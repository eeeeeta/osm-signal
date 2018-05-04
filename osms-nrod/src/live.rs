use ntrod_types::movements::{Activation, Cancellation, Movement, Record, MvtBody, EventType};
use ntrod_types::reference::CorpusEntry;
use osms_db::ntrod::types::*;
use osms_db::db::{DbType, InsertableDbType, GenericConnection};

type Result<T> = ::std::result::Result<T, ::failure::Error>;

pub fn process_ntrod_event<T: GenericConnection>(conn: &T, r: Record) -> Result<()> {
    let Record { header, body } = r;
    debug!("Processing message type {} from system {} (source {})",
           header.msg_type, header.source_system_id, header.original_data_source);
    match body {
        MvtBody::Activation(a) => process_activation(conn, a)?,
        MvtBody::Cancellation(a) => process_cancellation(conn, a)?,
        MvtBody::Movement(a) => process_movement(conn, a)?,
        _ => {
            warn!("Don't know/care about this type of message yet!");
            bail!("Unimplemented message type");
        }
    }
    Ok(())
}
pub fn process_activation<T: GenericConnection>(conn: &T, a: Activation) -> Result<()> {
    debug!("Processing activation of train {}...", a.train_id);
    let scheds = Schedule::from_select(conn,
        "WHERE uid = $1 AND stp_indicator = $2 AND start_date = $3",
        &[&a.train_uid, &a.schedule_type, &a.schedule_start_date])?;
    if scheds.len() == 0 {
        warn!("Failed to find a schedule.");
        bail!("Failed to find a schedule (UID {}, start {}, stp_indicator {:?})",
              a.train_uid, a.schedule_start_date, a.schedule_type);
    }
    let mut auth_schedule: Option<Schedule> = None;
    for sched in scheds {
        if !sched.is_authoritative(conn, a.origin_dep_timestamp.date())? {
            debug!("Schedule #{} is superseded.", sched.id);
        }
        else {
            if auth_schedule.is_some() {
                error!("Schedules #{} and #{} are both authoritative!",
                       sched.id, auth_schedule.as_ref().unwrap().id);
                bail!("Two authoritative schedules");
            }
            auth_schedule = Some(sched);
        }
    }
    let auth_schedule = if let Some(sch) = auth_schedule {
        sch
    }
    else {
        error!("No schedules are authoritative (UID {}, start {}, stp_indicator {:?})",
        a.train_uid, a.schedule_start_date, a.schedule_type);
        bail!("No authoritative schedules");
    };
    let train = Train {
        id: -1,
        parent_sched: auth_schedule.id,
        trust_id: a.train_id,
        date: a.origin_dep_timestamp.date(),
        signalling_id: a.schedule_wtt_id,
        cancelled: false,
        terminated: false
    };
    let id = train.insert_self(conn)?;
    debug!("Inserted train as #{}", id);
    Ok(())
}
pub fn process_cancellation<T: GenericConnection>(conn: &T, c: Cancellation) -> Result<()> {
    debug!("Processing cancellation of train {}...", c.train_id);
    conn.execute("UPDATE trains SET cancelled = true WHERE trust_id = $1", &[&c.train_id])?;
    debug!("Train cancelled.");
    Ok(())
}
pub fn process_movement<T: GenericConnection>(conn: &T, m: Movement) -> Result<()> {
    debug!("Processing movement of train {} at STANOX {}...", m.train_id, m.loc_stanox);
    if m.train_terminated {
        debug!("Train has terminated.");
        conn.execute("UPDATE trains SET terminated = true WHERE trust_id = $1", &[&m.train_id])?;
    }
    if m.offroute_ind {
        debug!("Train #{} off route.", m.train_id);
        return Ok(());
    }
    let trains = Train::from_select(conn, "WHERE trust_id = $1", &[&m.train_id])?;
    let train = match trains.into_iter().nth(0) {
        Some(t) => t,
        None => bail!("No train found for ID {}", m.train_id)
    };
    let entries = CorpusEntry::from_select(conn, "WHERE stanox = $1 AND tiploc IS NOT NULL",
                                           &[&m.loc_stanox])?;
    let tiploc = match entries.into_iter().nth(0) {
        Some(c) => c.tiploc.unwrap(),
        None => {
            debug!("No TIPLOC found for STANOX {}", m.loc_stanox);
            return Ok(());
        }
    };
    debug!("Mapped STANOX {} to TIPLOC {}", m.loc_stanox, tiploc);
    let mvts = ScheduleMvt::from_select(conn, "WHERE parent_sched = $1 AND tiploc = $2", &[&train.parent_sched, &tiploc])?;
    let mut did_something = false;
    for mvt in mvts {
        match (mvt.action, m.event_type) {
            (0, EventType::Arrival) => {},
            (0, EventType::Destination) => {},
            (2, EventType::Arrival) => {},
            (1, EventType::Departure) => {}
            _ => continue
        }
        let tmvt = TrainMvt {
            id: -1,
            parent_train: train.id,
            parent_mvt: mvt.id,
            time: m.actual_timestamp.time(),
            source: "TRUST".into()
        };
        let id = tmvt.insert_self(conn)?;
        did_something = true;
        debug!("Registered train movement #{}.", id);
    }
    if !did_something {
        bail!("didn't do anything useful!");
    }
    debug!("Train movement processed.");
    Ok(())
}
