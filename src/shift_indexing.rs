#![allow(warnings)]
use crate::GenResult;
use lopdf::Document;
use regex::Regex;
use std::path::PathBuf;
use time::macros::format_description;
use time::{error, format_description, Time};

trait StrTime {
    fn string_to_time(&self) -> Result<Time, error::Parse>;
}

impl StrTime for String {
    fn string_to_time(&self) -> Result<Time, error::Parse> {
        let format = format_description!("[hour]:[minute]");
        Ok(Time::parse(self, format)?)
    }
}

enum ShiftValid {
    Weekdagen,
    Zaterdag,
    Zondag,
}

enum ShiftType {
    Vroeg,
    Tussen,
    Gebroken {
        start_break: Option<Time>,
        end_break: Option<Time>,
    }, // If one is none, it means it's half of a broken shift
    Laat,
}

enum JobDrivingType {
    Lijn(u32),
    Mat,
}

enum JobMessageType {
    Meenemen { dienstnummers: Vec<u32> },
    Passagieren { dienstnummer: u32, omloop: String },
    BusOp { lijn: u32 },
    NeemBus { bustype: String },
    Other(String),
}

enum JobType {
    Rijden { drive_type: JobDrivingType },
    Pauze,
    Onderbreking,
    OpAfstap,
    RijklaarMaken,
    StallenAfmelden,
    Melding { message: JobMessageType },
    Temp,
}

struct ShiftJob {
    job_type: JobType,
    start: Option<Time>,
    end: Option<Time>,
    start_location: Option<String>,
    end_location: Option<String>, // If none, it's the same as start
    omloop: Option<u32>,
    rit: Option<u32>,
}

struct Shift {
    shift_nr: String,
    start_time: Time,
    end_time: Time,
    valid_on: ShiftValid,
    location: String,
    shift_type: ShiftType,
    job: Vec<ShiftJob>,
}

pub fn read_pdf_stream(pdf_path: PathBuf) -> GenResult<String> {
    let doc = Document::load(pdf_path)?;
    let pages = doc.get_pages();
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
                return Ok(find_text_and_coordinate(stream_string)?.first().unwrap().0.clone());
            }
            _ => {
                println!("Unexpected type for Contents on page {}", page_number);
            }
        }
        break;
    }
    Ok("Nothing found".to_string())
}

fn find_text_and_coordinate(page_stream: String) -> GenResult<Vec<(String, f32)>> {
    let re = Regex::new(r"\((.*?)\)").unwrap(); // Match text inside parentheses
    let mut line_elements: Vec<(String, (f32, f32))> = vec![];
    for (line_number, line) in page_stream.clone().lines().enumerate() {
        for cap in re.captures_iter(line) {
            let mut coordinate_split = page_stream
                .lines()
                .nth(line_number - 1)
                .unwrap()
                .split_ascii_whitespace();
            let coordinate: (f32, f32) = (
                coordinate_split.next().unwrap().parse().unwrap(),
                coordinate_split.next().unwrap().parse().unwrap(),
            );

            // println!(
            //     "Line {}: {} op positie {:?}",
            //     line_number + 1,
            //     &cap[1],
            //     coordinate
            // );
            line_elements.push((cap[1].to_string(), coordinate));
            if line.contains("Ingangsdatum"){
                let line = &cap[1];
                let start_date = line.split("Ingangsdatum ").last().unwrap();
                return Ok(vec![(start_date.to_string(),0.0)]);
            }
        }
    }
    get_line_element(line_elements)?;
    Ok(vec![("sad".to_string(), 1.0)])
}

fn get_line_element(items: Vec<(String, (f32, f32))>) -> GenResult<()> {
    let mut last_y = items.first().unwrap().1 .1;
    let mut lijn: Option<_> = None;
    let mut omloop: Option<_> = None;
    let mut rit: Option<_> = None;
    let mut start: Option<_> = None;
    let mut van: Option<_> = None;
    let mut naar: Option<_> = None;
    let mut eind: Option<_> = None;
    for item in items {
        if last_y != item.1 .1 {
            println!("Job gevonden!\nLijn {lijn:?}, omloop {omloop:?}, rit {rit:?}, van {van:?}, naar {naar:?}, begint om {start:?} en stopt om {eind:?}");
            lijn = None;
            omloop = None;
            rit = None;
            start = None;
            van = None;
            naar = None;
            eind = None;
        }
        match item.1 .0 {
            83.0..=92.0 => lijn = Some(item.0),
            200.0..=280.0 => omloop = Some(item.0),
            300.0..=350.0 => rit = Some(item.0),
            350.0..=390.0 => start = Some(item.0),
            400.0..=420.0 => van = Some(item.0),
            450.0..=480.0 => naar = Some(item.0),
            490.0.. => eind = Some(item.0),
            _ => (),
        }
        last_y = item.1 .1;
    }
    Ok(())
}

fn job_creator(
    lijn: Option<String>,
    omloop: Option<String>,
    rit: Option<String>,
    start: Option<String>,
    eind: Option<String>,
    van: Option<String>,
    naar: Option<String>,
) -> GenResult<()> {
    let job_drive_type;
    let job_type;
    if let Some(lijn_string) = lijn {
        if lijn_string == "MAT" {
            job_drive_type = Some(JobDrivingType::Mat);
        } else if let Ok(lijn_parse) = lijn_string.parse::<u32>() {
            job_drive_type = Some(JobDrivingType::Lijn(lijn_parse));
        } else {
            job_drive_type = None;
            let lijn_first_word = lijn_string.split_whitespace().next().unwrap().to_lowercase();
            let first_word_str = lijn_first_word.as_str();
            let message = match lijn_first_word.as_str() {
                "neem" => JobMessageType::NeemBus {
                    bustype: lijn_string.replace("neem ", ""),
                },
                "bus" => JobMessageType::BusOp {
                    lijn: lijn_string.replace("Bus op lijn ", "").parse()?,
                },
                "pod" => JobMessageType::NeemBus {
                    bustype: lijn_string,
                },
                "pass" => {
                    let lijn_string_split = lijn_string.replace("Pass met ", "");
                    let mut dienst_omloop_split = lijn_string_split.split_whitespace().next().unwrap().split('/');
                    JobMessageType::Passagieren {
                        dienstnummer: dienst_omloop_split.next().unwrap().parse().unwrap(),
                        omloop: dienst_omloop_split.next().unwrap().to_string(),
                    }
                }
                "meenemen" => JobMessageType::Other(lijn_string),
                _ => JobMessageType::Other(lijn_string),
            };
            job_type = message;
        }
    }
    Ok(())
}
