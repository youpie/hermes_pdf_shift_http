#![allow(warnings)]
use crate::GenResult;
use actix_web::web::get;
use lopdf::Document;
use regex::Regex;
use serde::Serialize;
use std::future;
use std::path::PathBuf;
use thiserror::Error;
use time::format_description::BorrowedFormatItem;
use time::macros::format_description;
use time::{Date, Time, error, format_description};

const DATE_FORMAT: &[BorrowedFormatItem<'_>] = format_description!["[day]-[month]-[year]"];

trait StrTime {
    fn string_to_time(&self) -> Result<Time, error::Parse>;
}

impl StrTime for String {
    fn string_to_time(&self) -> Result<Time, error::Parse> {
        let format = format_description!("[hour]:[minute]");
        Ok(Time::parse(self, format)?)
    }
}

#[derive(Error, Debug, Serialize)]
enum ShiftParseError {
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
enum ShiftValid {
    Weekdays,
    Saturday,
    Sunday,
    Unknown,
}

#[derive(Debug, Serialize)]
enum ShiftType {
    Vroeg,
    Tussen,
    Dag,
    Gebroken {
        start_break: Option<Time>,
        end_break: Option<Time>,
    }, // If one is none, it means it's half of a broken shift
    Laat,
}

#[derive(Debug, Serialize)]
enum JobDrivingType {
    Lijn(u32),
    Mat,
}

#[derive(Debug, Serialize)]
enum JobMessageType {
    Meenemen { dienstnummers: Vec<u32> },
    Passagieren { dienstnummer: u32, omloop: String },
    BusOp { lijn: u32 },
    NeemBus { bustype: String },
    Other(String),
}

#[derive(Debug, Serialize)]
enum JobType {
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
struct ShiftJob {
    job_type: JobType,
    start: Option<Time>,
    end: Option<Time>,
    start_location: Option<String>,
    end_location: Option<String>, // If none, it's the same as start
    omloop: Option<usize>,
    rit: Option<usize>,
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

pub fn read_pdf_stream(pdf_path: PathBuf) -> GenResult<Vec<Shift>> {
    let doc = Document::load(pdf_path)?;
    let pages = doc.get_pages();
    let mut i = 0;
    let mut shifts: Vec<Shift> = vec![];
    for (&page_number, &page_id) in pages.iter() {
        let page_dict = doc.get_object(page_id)?.as_dict()?;
        let contents = page_dict.get(b"Contents")?;
        //println!("{:#?}", contents);
        match contents {
            lopdf::Object::Reference(r) => {
                let object = doc.get_object(*r)?.as_stream()?;
                let test = object.get_plain_content()?;
                let stream_string = String::from_utf8_lossy(&test).to_string();
                let stream_string = stream_string.replace("ET\n", "");
                let stream_string = stream_string.replace("BT\n", "");
                let stream_string = stream_string.replace("Td", "");
                let stream_string = stream_string.replace("Tj", "");
                let stream_string = stream_string.replace("Tf", "");
                // let stream = lopdf::Object::Stream(*object);
                //println!("Page {} stream: {}", page_number, stream_string);
                let offset = if i % 2 == 0 { 0.0 } else { 48.0 };
                shifts.push(parse_page(stream_string, offset, page_number)?);
            }
            _ => {
                println!("Unexpected type for Contents on page {}", page_number);
            }
        }
        i += 1;
    }
    Ok(shifts)
}


fn parse_page(page_stream: String, offset: f32, page_number: u32) -> GenResult<Shift> {
    let re = Regex::new(r"\((.*?)\)")?; // Match text inside parentheses
    let mut line_elements: Vec<(String, (f32, f32))> = vec![];
    let page_stream_clone = page_stream.clone();
    for (line_number, line) in page_stream_clone.lines().enumerate() {
        for cap in re.captures_iter(line) {
            let mut coordinate_split = page_stream
                .lines()
                .nth(line_number - 1)
                .ok_or(ShiftParseError::Option {
                    function: "line coordinates",
                    parsing_job: None,
                    line: None,
                })?
                .split_ascii_whitespace();
            let coordinate: (f32, f32) = (
                coordinate_split
                    .next()
                    .ok_or(ShiftParseError::Option {
                        function: "line x coordinate",
                        parsing_job: None,
                        line: None,
                    })?
                    .parse()?,
                coordinate_split
                    .next()
                    .ok_or(ShiftParseError::Option {
                        function: "line y coordinate",
                        parsing_job: None,
                        line: None,
                    })?
                    .parse()?,
            );

            // println!(
            //     "Line {}: {} op positie {:?}",
            //     line_number + 1,
            //     &cap[1],
            //     coordinate
            // );
            line_elements.push((cap[1].to_string(), coordinate));
        }
    }

    let shift = get_line_element(line_elements, offset, page_number)?;
    Ok(shift)
}

fn get_line_element(
    items: Vec<(String, (f32, f32))>,
    offset: f32,
    page_number: u32,
) -> GenResult<Shift> {
    let mut line_errors: Vec<ShiftParseError> = vec![];

    let mut last_y = items
        .first()
        .ok_or(ShiftParseError::Option {
            function: "first line",
            parsing_job: None,
            line: None,
        })?
        .1
        .1;
    let mut lijn: Option<String> = None;
    let mut omloop: Option<_> = None;
    let mut rit: Option<_> = None;
    let mut start: Option<_> = None;
    let mut van: Option<_> = None;
    let mut naar: Option<_> = None;
    let mut eind: Option<_> = None;
    let mut start_date = Date::from_calendar_date(2025, time::Month::June, 29)?;
    let mut valid_on = ShiftValid::Unknown;
    let mut shift_number = String::new();
    let mut jobs = vec![];
    for item in items {
        match get_line_information(
            &mut lijn,
            &mut omloop,
            &mut rit,
            &mut start,
            &mut van,
            &mut naar,
            &mut eind,
            &mut jobs,
            &mut start_date,
            &mut valid_on,
            &mut shift_number,
            last_y,
            item.1.1,
            item.1.0,
            offset,
            page_number,
            item.0,
        ) {
            Ok(_) => (),
            Err(err) => line_errors.push(err),
        };
        last_y = item.1.1;
    }
    Ok(Shift {
        shift_nr: shift_number.to_string(),
        valid_on: valid_on,
        location: "todo".to_string(),
        shift_type: None,
        job: jobs,
        starting_date: start_date,
        parse_error: None,
    })
}

fn get_line_information(
    lijn: &mut Option<String>,
    omloop: &mut Option<String>,
    rit: &mut Option<String>,
    start: &mut Option<String>,
    van: &mut Option<String>,
    naar: &mut Option<String>,
    eind: &mut Option<String>,
    jobs: &mut Vec<ShiftJob>,
    start_date: &mut Date,
    valid_on: &mut ShiftValid,
    shift_number: &mut String,
    last_y: f32,
    current_y: f32,
    current_x: f32,
    offset: f32,
    page_number: u32,
    line: String,
) -> Result<(),ShiftParseError> {
    let lijn_lower = 83.0 - offset;
    let lijn_upper = 150.0 - offset;
    let omloop_lower = 200.0 - offset;
    let omloop_upper = 280.0 - offset;
    let rit_lower = 300.0 - offset;
    let rit_upper = 350.0 - offset;
    let start_lower = 350.0 - offset;
    let start_upper = 390.0 - offset;
    let van_lower = 400.0 - offset;
    let van_upper = 420.0 - offset;
    let naar_lower = 450.0 - offset;
    let naar_upper = 480.0 - offset;
    let eind_lower = 490.0 - offset;
    if current_y < 40.0 || current_y > 720.0 {
        if let Some(metadata) = lijn.clone() {
            identify_metadata(
                &mut *start_date,
                &mut *valid_on,
                &mut *shift_number,
                metadata,
            )
            .ok_or(ShiftParseError::MetadataFailure {
                page_number,
                line: None,
            })?;
        }
    } else if last_y != current_y {
        // println!("Job gevonden!\nLijn {lijn:?}, omloop {omloop:?}, rit {rit:?}, van {van:?}, naar {naar:?}, begint om {start:?} en stopt om {eind:?}");
        let job = job_creator(
            lijn.clone(),
            omloop.clone(),
            rit.clone(),
            start.clone(),
            eind.clone(),
            van.clone(),
            naar.clone(),
        )?;
        //println!("{:?}", &job);
        jobs.push(job);
        *lijn = None;
        *omloop = None;
        *rit = None;
        *start = None;
        *van = None;
        *naar = None;
        *eind = None;
    }
    if current_x >= lijn_lower && current_x <= lijn_upper {
        *lijn = Some(line);
    } else if current_x >= omloop_lower && current_x <= omloop_upper {
        *omloop = Some(line);
    } else if current_x >= rit_lower && current_x <= rit_upper {
        *rit = Some(line);
    } else if current_x >= start_lower && current_x <= start_upper {
        *start = Some(line);
    } else if current_x >= van_lower && current_x <= van_upper {
        *van = Some(line);
    } else if current_x >= naar_lower && current_x <= naar_upper {
        *naar = Some(line);
    } else if current_x >= eind_lower {
        *eind = Some(line);
    }
    Ok(())
}

fn identify_metadata(
    start_date: &mut Date,
    valid_on: &mut ShiftValid,
    shift_number: &mut String,
    metadata: String,
) -> Option<()> {
    if metadata.contains("Ingangsdatum ") {
        *start_date = Date::parse(metadata.split("Ingangsdatum ").last()?, DATE_FORMAT).ok()?;
    } else if metadata.contains("Dienst ") {
        *shift_number = metadata.split("Dienst ").last()?.to_owned();
    } else if metadata.contains("MA/DI/WO/DO/VR") {
        *valid_on = ShiftValid::Weekdays;
    } else if metadata.contains("ZA") {
        *valid_on = ShiftValid::Saturday;
    } else if metadata.contains("ZO") {
        *valid_on = ShiftValid::Sunday;
    }
    Some(())
}

fn job_creator(
    lijn: Option<String>,
    omloop: Option<String>,
    rit: Option<String>,
    start: Option<String>,
    eind: Option<String>,
    van: Option<String>,
    naar: Option<String>,
) -> Result<ShiftJob,ShiftParseError> {
    let mut omloop_number = None;
    let mut job_type = JobType::Unknown;
    let mut rit_number = None;
    let mut start_time: Option<Time> = None;
    let mut end_time = None;
    if let Some(lijn_string) = lijn {
        if lijn_string == "MAT" {
            job_type = JobType::Rijden {
                drive_type: JobDrivingType::Mat,
            };
        } else if lijn_string == "Pauze" {
            job_type = JobType::Pauze;
        } else if let Ok(lijn_parse) = lijn_string.parse::<u32>() {
            job_type = JobType::Rijden {
                drive_type: JobDrivingType::Lijn(lijn_parse),
            };
        } else {
            let message = match message_type_finder(lijn_string.clone()) {
                Some(message) => message,
                None => JobMessageType::Other(lijn_string),
            };
            job_type = JobType::Melding { message };
        }
    }
    if let Some(rit_string) = rit {
        rit_number = rit_string.parse::<usize>().ok();
    }
    if let Some(start_string) = start {
        to_iso8601(start_string, "Start time")?;
    }
    if let Some(end_string) = eind {
        to_iso8601(end_string, "End time")?;
    }
    if let Some(omloop_string) = omloop {
        match omloop_string.as_ref() {
            "Onderbreking" => job_type = JobType::Onderbreking,
            "Loop/Reis" => job_type = JobType::LoopReis,
            "Rijklaar maken" => job_type = JobType::RijklaarMaken,
            "Bus stallen/afm" => job_type = JobType::StallenAfmelden,
            "Reserve" => job_type = JobType::Reserve,
            _ => omloop_number = omloop_string.parse::<usize>().ok(),
        };
    }

    Ok(ShiftJob {
        job_type,
        start: start_time,
        end: end_time,
        start_location: van,
        end_location: naar,
        omloop: omloop_number,
        rit: rit_number,
    })
}

fn to_iso8601(time_string: String, job_name: &str) -> Result<Option<Time>,ShiftParseError> {
    let mut time_split = time_string.split(":").into_iter();
    let hour_noniso = time_split
        .next()
        .ok_or(ShiftParseError::Option {
            function: "Time hour",
            parsing_job: Some(job_name.to_string()),
            line: Some(time_string.clone()),
        })?
        .parse::<u8>().map_err(|err| ShiftParseError::GenericShiftError { page_number: 
            0, error: err.to_string(), line: Some(time_string.clone()) })?;
    let minute = time_split
        .next()
        .ok_or(ShiftParseError::Option {
            function: "Time minute",
            parsing_job: Some(job_name.to_string()),
            line: Some(time_string.clone()),
        })?
        .parse::<u8>().map_err(|err| ShiftParseError::GenericShiftError { page_number: 
            0, error: err.to_string(), line: Some(time_string.clone()) })?;
    let hour_iso = match hour_noniso {
        24.. => hour_noniso - 24,
        _ => hour_noniso,
    };
    Ok(Time::from_hms(hour_iso, minute, 0).ok())
}

fn message_type_finder(lijn_string: String) -> Option<JobMessageType> {
    let lijn_first_word = lijn_string.split_whitespace().next()?.to_lowercase();
    let first_word_str = lijn_first_word.as_str();
    let message = match lijn_first_word.as_str() {
        "neem" => JobMessageType::NeemBus {
            bustype: lijn_string.replace("neem ", ""),
        },
        "bus" => JobMessageType::BusOp {
            lijn: lijn_string.replace("Bus op lijn ", "").parse().ok()?,
        },
        "pod" => JobMessageType::NeemBus {
            bustype: lijn_string,
        },
        "pass" => {
            let lijn_string_split = lijn_string.replace("Pass met ", "");
            let mut dienst_omloop_split = lijn_string_split.split_whitespace().next()?.split('/');
            JobMessageType::Passagieren {
                dienstnummer: dienst_omloop_split.next()?.parse().ok()?,
                omloop: dienst_omloop_split.next()?.to_string(),
            }
        }
        "meenemen" => JobMessageType::Other(lijn_string),
        _ => JobMessageType::Other(lijn_string),
    };
    Some(message)
}
