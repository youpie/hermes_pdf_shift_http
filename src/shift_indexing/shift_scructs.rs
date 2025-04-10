use serde::Serialize;
use thiserror::Error;
use time::{Date, Time};

#[derive(Error, Debug, Serialize)]
pub enum ShiftParseError {
    #[error("Shift on page {page_number} had a generic error{error_string}\nline: {line:?}",error_string = error.to_string())]
    GenericShiftError {
        page_number: u32,
        error: String,
        line: Option<String>,
    },
    #[error("Failed to parse metadata on page {page_number}\nline: {line:?}")]
    MetadataFailure {
        page_number: u32,
        line: Option<String>,
    },
    #[error("{function}: Unwrapped an option while parsing {parsing_job:?}\nline: {line:?}")]
    Option {
        function: &'static str,
        parsing_job: Option<String>,
        line: Option<String>,
    },
}

#[derive(Debug, Serialize)]
pub enum ShiftValid {
    Weekdays,
    Saturday,
    Sunday,
    Unknown,
}

#[derive(Debug, Serialize)]
pub enum ShiftType {
    Vroeg,
    Tussen,
    Dag,
    Gebroken {
        start_break: Option<Time>,
        end_break: Option<Time>,
    }, // If one is none, it means it's half of a broken shift
    Laat,
}

#[derive(Debug, Serialize, PartialEq)]
pub enum JobDrivingType {
    Lijn(u32),
    Mat,
}

#[derive(Debug, Serialize, PartialEq)]
pub enum JobMessageType {
    Meenemen { dienstnummers: Vec<u32> },
    Passagieren { dienstnummer: u32, omloop: String },
    BusOp { lijn: u32 },
    NeemBus { bustype: String },
    Other(String),
}

#[derive(Debug, Serialize,PartialEq)]
pub enum JobType {
    Rijden { drive_type: JobDrivingType },
    Pauze,
    Onderbreking,
    OpAfstap,
    RijklaarMaken,
    StallenAfmelden,
    Melding { message: JobMessageType },
    LoopReis,
    Reserve,
    Unknown,
}

#[derive(Debug, Serialize)]
pub struct ShiftJob {
    pub job_type: JobType,
    pub start: Option<Time>,
    pub end: Option<Time>,
    pub start_location: Option<String>,
    pub end_location: Option<String>, // If none, it's the same as start
    pub omloop: Option<usize>,
    pub rit: Option<usize>,
}

impl ShiftJob {
    pub fn empty(&self) -> bool {
        if (self.job_type == JobType::Unknown && self.start.is_none() && self.end.is_none() && self.start_location.is_none() && self.end_location.is_none()) {return true;}
        false
    }
}

#[derive(Debug, Serialize)]
pub struct Shift {
    pub shift_nr: String,
    pub valid_on: ShiftValid,
    pub location: String,
    pub shift_type: Option<ShiftType>,
    pub job: Vec<ShiftJob>,
    pub starting_date: Date,
    pub parse_error: Option<Vec<ShiftParseError>>,
}
