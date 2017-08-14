#[macro_use] extern crate error_chain;
extern crate osm_signal;
extern crate postgres;
extern crate geo;
extern crate indicatif;
extern crate postgis;
extern crate ordered_float;

static ACCESS_TOKEN: &str = "[REDACTED]";
static DATABASE_URL: &str = "postgresql://eeeeeta@127.0.0.1/osm";
static HUXLEY_URL: &str = "https://huxley.apphb.com";

use std::collections::{HashSet, BTreeSet, HashMap};
use postgres::{Connection, GenericConnection, TlsMode};
use postgres::rows::Row;
use postgres::types::ToSql;
use postgis::ewkb::{Point, LineString, LineStringT};
use osm_signal::*;
use ordered_float::OrderedFloat;
use indicatif::ProgressBar;
mod errors {
    error_chain! {
        links {
            Rail(::osm_signal::errors::RailError, ::osm_signal::errors::RailErrorKind);
        }
        foreign_links {
            Io(::std::io::Error);
            Postgres(::postgres::error::Error);
            PostgresConnect(::postgres::error::ConnectError);
        }
    }
}
use errors::*;
fn rail() -> Result<()> {
    let mut cli = RailClient::new(ACCESS_TOKEN, HUXLEY_URL)?;
    let mut servs = HashMap::new();
    let sb = cli.station_board("CLJ", 90)?;
    for serv in sb.train_services {
        if let Some(id) = serv.id {
            if let Some(rsid) = serv.rsid {
                servs.insert(rsid, id);
            }
        }
    }
    println!("[+] {} train services", servs.len());
    for (rsid, sid) in servs {
        let serv = cli.service(&sid)?;
        println!("{:#?}", serv);
    }
    Ok(())
}
#[derive(Debug, Clone)]
pub struct Node {
    id: i32,
    location: Point,
    distance: f32,
    parent: Option<i32>,
    visited: bool,
    graph_part: i32,
    parent_geom: Option<LineString>
}
impl Node {
    pub fn make_table<T: GenericConnection>(conn: &T) -> Result<()> {
        conn.execute(r#"
CREATE TABLE IF NOT EXISTS nodes (
id SERIAL PRIMARY KEY,
location geometry UNIQUE NOT NULL,
distance REAL NOT NULL DEFAULT 'Infinity',
parent INT,
visited BOOL NOT NULL DEFAULT false,
graph_part INT NOT NULL DEFAULT 0,
parent_geom geometry
);"#, &[])?;
        Ok(())
    }
    pub fn from_select<T: GenericConnection>(conn: &T, where_clause: &str, args: &[&ToSql]) -> Result<Vec<Self>> {
        let query = format!("SELECT * FROM nodes {}", where_clause);
        let qry = conn.query(&query, args)?;
        let mut ret = vec![];
        for row in &qry {
            ret.push(Self::from_row(&row));
        }
        Ok(ret)
    }
    pub fn from_row(row: &Row) -> Self {
        Self {
            id: row.get(0),
            location: row.get(1),
            distance: row.get(2),
            parent: row.get(3),
            visited: row.get(4),
            graph_part: row.get(5),
            parent_geom: row.get(6)
        }
    }
    pub fn insert<T: GenericConnection>(conn: &T, location: Point, distance: f32) -> Result<()> {
        conn.execute("INSERT INTO nodes (location, distance, parent, visited) VALUES ($1, $2) ON CONFLICT DO NOTHING",
                     &[&location, &distance])?;
        Ok(())
    }
}
#[derive(Debug, Clone)]
pub struct Link {
    p1: i32,
    p2: i32,
    way: LineString,
    distance: f32
}
impl Link {
    pub fn make_table<T: GenericConnection>(conn: &T) -> Result<()> {
        conn.execute(r#"
CREATE TABLE IF NOT EXISTS links (
p1 INT NOT NULL,
p2 INT NOT NULL,
way geometry NOT NULL,
distance REAL NOT NULL
);"#, &[])?;
        Ok(())
    }
        pub fn from_select<T: GenericConnection>(conn: &T, where_clause: &str, args: &[&ToSql]) -> Result<Vec<Self>> {
        let query = format!("SELECT p1, p2, way, distance FROM links {}", where_clause);
        let qry = conn.query(&query, args)?;
        let mut ret = vec![];
        for row in &qry {
            ret.push(Self::from_row(&row));
        }
        Ok(ret)
    }
    pub fn from_row(row: &Row) -> Self {
        Self {
            p1: row.get(0),
            p2: row.get(1),
            way: row.get(2),
            distance: row.get(3)
        }
    }
    pub fn insert<T: GenericConnection>(&self, conn: &T) -> Result<()> {
        conn.execute("INSERT INTO links (p1, p2, way, distance) VALUES ($1, $2, $3, $4)",
                     &[&self.p1, &self.p2, &self.way, &self.distance])?;
        Ok(())
    }
}

fn count<T: GenericConnection>(conn: &T, details: &str, args: &[&ToSql]) -> Result<i64> {
    Ok(conn.query(&format!("SELECT COUNT(*) {}", details), args)?.into_iter()
        .nth(0)
        .ok_or("Count query failed")?
        .get(0))
}
fn osm() -> Result<()> {
    let conn = Connection::connect(DATABASE_URL, TlsMode::None)?;
    println!("[+] Creating tables...");
    Node::make_table(&conn)?;
    Link::make_table(&conn)?;
    let cnt = count(&conn, "FROM planet_osm_line WHERE railway IS NOT NULL", &[])?;
    println!("[+] {} ways in database", cnt);
    let mut nodes = count(&conn, "FROM nodes", &[])?;
    if nodes == 0 {
        println!("[+] Making nodes...");
        let bar = ProgressBar::new(cnt as _);
        let trans = conn.transaction()?;
        for row in &trans.query("SELECT ST_StartPoint(way), ST_EndPoint(way) FROM planet_osm_line WHERE railway IS NOT NULL", &[])? {
            Node::insert(&trans, row.get(0), ::std::f32::INFINITY)?;
            Node::insert(&trans, row.get(1), ::std::f32::INFINITY)?;
            bar.inc(1);
        }
        trans.commit()?;
        bar.finish();
        nodes = count(&conn, "FROM nodes", &[])?;
    }
    println!("[+] {} nodes in database", nodes);
    let mut links = count(&conn, "FROM links", &[])?;
    if links == 0 {
        println!("[+] Forming links...");
        let bar = ProgressBar::new(nodes as _);
        let trans = conn.transaction()?;
        for node in Node::from_select(&trans, "", &[])? {
            for row in &trans.query("SELECT way, CAST(ST_Length(way) AS REAL), id FROM planet_osm_line INNER JOIN nodes ON ST_EndPoint(planet_osm_line.way) = nodes.location WHERE railway IS NOT NULL AND ST_Intersects(ST_StartPoint(way), $1)", &[&node.location])? {
                let link = Link { p1: node.id, p2: row.get(2), way: row.get(0), distance: row.get(1) };
                link.insert(&trans)?;
            }
            bar.inc(1);
        }
        trans.commit()?;
        bar.finish();
        links = count(&conn, "FROM links", &[])?;
    }
    println!("[+] {} links in database", links);
    let unclassed_nodes = count(&conn, "FROM nodes WHERE graph_part = 0", &[])?;
    let mut cur_graph_part = 1;
    if unclassed_nodes > 0 {
        println!("[+] Separating nodes...");
        let bar = ProgressBar::new_spinner();
        let trans = conn.transaction()?;
        let mut total = 0;
        loop {
            let vec = Node::from_select(&trans, "WHERE graph_part = 0 LIMIT 1", &[])?;
            if vec.len() == 0 {
                break;
            }
            bar.set_position(0);
            for node in vec {
                let mut part_of_this = HashSet::new();
                part_of_this.insert(node.id);
                let mut current_roots = HashSet::new();
                current_roots.insert(node.id);
                loop {
                    bar.set_message(&format!("Considering graph part {} (this = {}, total = {}/{})", cur_graph_part, part_of_this.len(), total+part_of_this.len(), nodes));
                    if current_roots.len() == 0 {
                        total += part_of_this.len();
                        break;
                    }
                    for root in ::std::mem::replace(&mut current_roots, HashSet::new()) {
                        for link in Link::from_select(&trans, "WHERE p1 = $1 OR p2 = $1", &[&root])? {
                            let other_end = if link.p1 == root { link.p2 } else { link.p1 };
                            if other_end != root && part_of_this.insert(other_end) {
                                current_roots.insert(other_end);
                            }
                        }
                    }
                }
                let part_of_this = part_of_this.into_iter().collect::<Vec<_>>();
                trans.execute("UPDATE nodes SET graph_part = $1 WHERE id = ANY($2)", &[&cur_graph_part, &part_of_this])?;
            }
            cur_graph_part += 1;
        }
        trans.commit()?;
        bar.finish();
    }
    else {
        for node in Node::from_select(&conn, "ORDER BY graph_part DESC LIMIT 1", &[])? {
            cur_graph_part = node.graph_part;
        }
    }
    println!("[+] All nodes separated, {} graph parts", cur_graph_part);
    conn.execute("UPDATE nodes SET distance = 'Infinity', visited = false, parent = NULL, parent_geom = NULL", &[])?;
    const starting_node: i32 = 45896;
    const goal_node: i32 = 45282;
    conn.execute("UPDATE nodes SET distance = 0 WHERE id = $1", &[&starting_node])?;
    let trans = conn.transaction()?;
    println!("[+] Navigating from {} to {}", starting_node, goal_node);
    let mut cur = Node::from_select(&trans, "WHERE id = $1", &[&starting_node])?.into_iter().nth(0)
        .ok_or("Starting node does not exist!")?;
    let dest = Node::from_select(&trans, "WHERE id = $1 AND graph_part = $2", &[&goal_node, &cur.graph_part])?.into_iter().nth(0)
        .ok_or("Finishing node does not exist, or is not in the same graph part as the starting node")?;
    let nodes_in_gp = count(&trans, "FROM nodes WHERE graph_part = $1", &[&cur.graph_part])?;
    let bar = ProgressBar::new(nodes_in_gp as _);
    let mut distance: f32 = 0.0;
    'outer: loop {
        assert!(cur.distance != ::std::f32::INFINITY);
        bar.set_message(&format!("Considering node {}", cur.id));
        bar.inc(1);
        let links = Link::from_select(&trans, "WHERE p1 = $1 OR p2 = $1", &[&cur.id])?;
        for link in links {
            let tent_dist = link.distance + cur.distance;
            let other_end = if link.p1 == cur.id { link.p2 } else { link.p1 };
            for row in &trans.query("UPDATE nodes SET distance = CASE WHEN distance > $1 THEN $1 ELSE distance END WHERE id = $2 AND visited = false RETURNING id", &[&tent_dist, &other_end])? {
                let id: i32 = row.get(0);
                trans.execute("UPDATE nodes SET parent = $1, parent_geom = $2 WHERE id = $3", &[&cur.id, &link.way, &id])?;
                if id == dest.id {
                    distance = tent_dist;
                    break 'outer;
                }
            }
        }
        trans.execute("UPDATE nodes SET visited = true WHERE id = $1", &[&cur.id])?;
        let next = Node::from_select(&trans, "WHERE visited = false AND graph_part = $1 ORDER BY distance ASC LIMIT 1", &[&cur.graph_part])?;
        for node in next {
            cur = node;
            continue 'outer;
        }
        break;
    }
    trans.commit()?;
    bar.finish();
    if distance == 0.0 {
        println!("[+] It is unpossible! No path found...");
        return Ok(());
    }
    println!("[+] Djikstra's algorithm complete! Distance = {}", distance);
    println!("[+] Producing actual path...");
    let mut ret = vec![];
    let mut cur_node = Node::from_select(&conn, "WHERE id = $1 AND graph_part = $2", &[&goal_node, &cur.graph_part])?.into_iter().nth(0).unwrap();
    loop {
        ret.push(cur_node.id);
        if cur_node.parent.is_none() {
            break;
        }
        let mut vec = Node::from_select(&conn, "WHERE id = $1", &[&cur_node.parent.unwrap()])?;
        cur_node = vec.remove(0);
    }
    println!("[+] Path is: {:?}", ret.iter().rev().collect::<Vec<_>>());
    Ok(())
}
quick_main!(osm);
