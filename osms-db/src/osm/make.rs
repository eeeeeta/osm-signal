use super::types::*;
use db::{GenericConnection, DbPool, DbType, InsertableDbType};
use postgis::ewkb::{Point, Polygon};
use geo;
use util::*;
use errors::*;
use std::collections::{HashMap, HashSet};
use std::thread;
use std::sync::atomic::{Ordering, AtomicUsize};
use std::sync::Arc;
use std::time::Instant;
use std::sync::mpsc::channel;

pub fn make_crossings<T: GenericConnection>(conn: &T) -> Result<()> {
    debug!("make_crossings: running...");
    let trans = conn.transaction()?;
    let mut processed_osm_ids = HashSet::new();
    let mut changed = 0;
    for row in &trans.query("SELECT osm_id, way, ST_Buffer(way::geography, 20), name FROM planet_osm_point WHERE railway = 'level_crossing'", &[])? {
        let (osm_id, way, area, name): (i64, Point, Polygon, Option<String>)
            = (row.get(0), row.get(1), row.get(2), row.get(3));
        if processed_osm_ids.insert(osm_id) {
            let (node_id, _) = Node::new_at_point(&trans, way.clone())?;
            let mut other_node_ids = vec![];
            for row in &trans.query("SELECT osm_id FROM planet_osm_point WHERE ST_Intersects(way, $1)",
                                    &[&area])? {
                let osm_id: i64 = row.get(0);
                if processed_osm_ids.insert(osm_id) {
                    other_node_ids.push(Node::new_at_point(&trans, way.clone())?.0);
                }
            }
            let lxing = Crossing { node_id, name, other_node_ids, area };
            lxing.insert_self(&trans)?;
            changed += 1;
            if (changed % 100) == 0 {
                debug!("make_crossings: made {} crossings", changed);
            }
        }
    }
    trans.commit()?;
    Ok(())
}
pub fn make_stations<T: GenericConnection>(conn: &T) -> Result<()> {
    use geo::algorithm::closest_point::ClosestPoint;
    let trans = conn.transaction()?;
    let mut areas: HashMap<String, (Polygon, Point)> = HashMap::new();
    for row in &trans.query(
        "SELECT ref, way, ST_Centroid(way)
         FROM planet_osm_polygon
         WHERE railway = 'station' AND ref IS NOT NULL", &[])? {

        areas.insert(row.get(0), (row.get(1), row.get(2)));
    }
    for row in &trans.query(
        "SELECT ref, ST_Buffer(way::geography, 50)::geometry, way
         FROM planet_osm_point
         WHERE railway = 'station' AND ref IS NOT NULL", &[])? {

        areas.insert(row.get(0), (row.get(1), row.get(2)));
    }
    debug!("make_stations: {} stations to process", areas.len());
    for (nr_ref, (poly, point)) in areas {
        let pt = geo::Point::from_postgis(&point);
        let (node, _) = Node::new_at_point(&trans, point.clone())?;
        let links = Link::from_select(&trans, "WHERE ST_Intersects(way, $1)", &[&poly])?;
        let trigd = links.len() != 0;
        for link in links {
            debug!("making new point for station {}", nr_ref);
            let geoway = geo::LineString::from_postgis(&link.way);

            let geocp = geoway.closest_point(&pt).ok_or("closest_point() returned None")?;
            let cp = geo_pt_to_postgis(geocp);

            let (end, trigd) = Node::new_at_point(&trans, cp)?;
            if !trigd {
                let links = Link::from_select(&trans, "WHERE p1 = $1 OR p2 = $1", &[&end])?;
                if links.len() != 0 {
                    bail!("Point ({},{}) didn't get connected to anything.",
                          geocp.lat(), geocp.lng());
                }
            }
            let connection = geo::LineString(vec![pt, geocp]);
            let link = Link {
                p1: node,
                p2: end,
                distance: 0.0,
                way: geo_ls_to_postgis(connection)
            };
            link.insert(&trans)?;
        }
        if !trigd {
            warn!("*** Station {} didn't connect to anything!", nr_ref);
        }
        Station::insert(&trans, &nr_ref, node, poly)?;
    }
    trans.commit()?;
    Ok(())
}

pub fn make_nodes<T: GenericConnection>(conn: &T) -> Result<()> {
    debug!("make_nodes: making nodes from OSM data...");
    let trans = conn.transaction()?;
    let mut compl = 0;
    for row in &trans.query("SELECT ST_StartPoint(way), ST_EndPoint(way)
                            FROM planet_osm_line WHERE railway IS NOT NULL", &[])? {
        Node::insert(&trans, row.get(0))?;
        Node::insert(&trans, row.get(1))?;
        compl += 1;
        if (compl % 1000) == 0 {
            debug!("make_nodes: completed {} rows", compl);
        }
    }
    trans.commit()?;
    debug!("make_nodes: complete!");
    Ok(())
}
pub fn make_links(pool: &DbPool, n_threads: usize) -> Result<()> {
    debug!("make_links: making links from OSM data...");
    let todo = count(&*pool.get().unwrap(), "FROM nodes WHERE processed = false", &[])?;
    debug!("make_links: {} nodes to make links for", todo);
    let done = Arc::new(AtomicUsize::new(0));
    let mut threads = vec![];
    let p = pool.clone();
    let (tx, rx) = channel::<Option<Link>>();
    let endthr = thread::spawn(move || {
        let db = p.get().unwrap();
        debug!("make_links: spawning inserter thread");
        while let Some(link) = rx.recv().unwrap() {
            link.insert(&*db).unwrap();
        }
        debug!("make_links: inserter thread done");
    });
    for n in 0..n_threads {
        debug!("make_links: spawning thread {}", n);
        let p = pool.clone();
        let d = done.clone();
        let tx = tx.clone();
        threads.push(thread::spawn(move || {
            let db = p.get().unwrap();
            loop {
                let trans = db.transaction().unwrap();
                let nodes = Node::from_select(&trans, "WHERE processed = false LIMIT 1
                                                       FOR UPDATE SKIP LOCKED", &[])
                    .unwrap();
                if nodes.len() == 0 {
                    debug!("make_links: thread {} done", n);
                    break;
                }
                for node in nodes {
                    let instant = Instant::now();
                    for row in &trans.query(
                        "SELECT way, CAST(ST_Length(way::geography, false) AS REAL), id
                         FROM planet_osm_line
                         INNER JOIN nodes ON ST_EndPoint(planet_osm_line.way) = nodes.location
                         WHERE railway IS NOT NULL AND ST_Intersects(ST_StartPoint(way), $1)",
                        &[&node.location]).unwrap() {
                        let link = Link { p1: node.id, p2: row.get(2), way: row.get(0), distance: row.get(1) };
                        tx.send(Some(link)).unwrap();
                    }
                    trans.execute("UPDATE nodes SET processed = true WHERE id = $1", &[&node.id])
                        .unwrap();
                    let now = Instant::now();
                    let dur = now.duration_since(instant);
                    let dur = dur.as_secs() as f64 + dur.subsec_nanos() as f64 * 1e-9;
                    let done = d.fetch_add(1, Ordering::SeqCst) + 1;
                    debug!("make_links: {} of {} nodes complete ({:.01}%) - time: {:.04}s", done, todo, (done as f64 / todo as f64) * 100.0, dur);
                }
                trans.commit().unwrap();
            }
        }));
    }
    for thr in threads {
        thr.join().unwrap();
    }
    tx.send(None).unwrap();
    endthr.join().unwrap();
    debug!("make_links: complete!");
    Ok(())
}
