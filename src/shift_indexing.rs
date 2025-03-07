#![allow(warnings)]
use crate::GenResult;
use lopdf::Document;
use regex::Regex;
use std::path::PathBuf;
use time::macros::format_description;
use time::{error, format_description, Date, Time};
use time::format_description::BorrowedFormatItem;

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

#[derive(Debug)]
enum ShiftValid {
    Weekdays,
    Saturday,
    Sunday,
    Unknown,
}

#[derive(Debug)]
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

#[derive(Debug)]
enum JobDrivingType {
    Lijn(u32),
    Mat,
}

#[derive(Debug)]
enum JobMessageType {
    Meenemen { dienstnummers: Vec<u32> },
    Passagieren { dienstnummer: u32, omloop: String },
    BusOp { lijn: u32 },
    NeemBus { bustype: String },
    Other(String),
}

#[derive(Debug)]
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

#[derive(Debug)]
struct ShiftJob {
    job_type: JobType,
    start: Option<Time>,
    end: Option<Time>,
    start_location: Option<String>,
    end_location: Option<String>, // If none, it's the same as start
    omloop: Option<usize>,
    rit: Option<usize>,
}

#[derive(Debug)]
pub struct Shift {
    pub shift_nr: String,
    pub valid_on: ShiftValid,
    pub location: String,
    pub shift_type: Option<ShiftType>,
    pub job: Vec<ShiftJob>,
    pub starting_date: Date,
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
                let offset = if i%2 == 0 {0.0} else {48.0};
                shifts.push(find_text_and_coordinate(stream_string,offset)?);
            }
            _ => {
                println!("Unexpected type for Contents on page {}", page_number);
            }
        }
        i += 1;
    }
    Ok(shifts)
}

fn find_text_and_coordinate(page_stream: String, offset: f32) -> GenResult<Shift> {
    let re = Regex::new(r"\((.*?)\)").unwrap(); // Match text inside parentheses
    let mut line_elements: Vec<(String, (f32, f32))> = vec![];
    let page_stream_clone = page_stream.clone();
    for (line_number, line) in page_stream_clone.lines().enumerate() {
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
        }
    }
    
    let shift = get_line_element(line_elements,offset)?;
    Ok(shift)
}

fn get_line_element(items: Vec<(String, (f32, f32))>, offset: f32) -> GenResult<Shift> {
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

    let mut last_y = items.first().unwrap().1 .1;
    let mut lijn: Option<String> = None;
    let mut omloop: Option<_> = None;
    let mut rit: Option<_> = None;
    let mut start: Option<_> = None;
    let mut van: Option<_> = None;
    let mut naar: Option<_> = None;
    let mut eind: Option<_> = None;
    let mut start_date= Date::from_calendar_date(2025, time::Month::June, 29).unwrap();
    let mut valid_on= ShiftValid::Unknown;
    let mut shift_number = String::new();
    let mut jobs = vec![];
    for item in items {
        if item.1.1 < 40.0 || item.1.1 > 720.0{
            if let Some(metadata) = lijn.clone() {
                if metadata.contains("Ingangsdatum "){
                    start_date = Date::parse(metadata.split("Ingangsdatum ").last().unwrap(),DATE_FORMAT).unwrap();
                }
                else if metadata.contains("Dienst "){
                    shift_number = metadata.split("Dienst ").last().unwrap().to_owned();
                }
                else if metadata.contains("MA/DI/WO/DO/VR"){
                    valid_on = ShiftValid::Weekdays;
                }
                else if metadata.contains("MA/DI/WO/DO/VR"){
                    valid_on = ShiftValid::Weekdays;
                }
                else if metadata.contains("ZA"){
                    valid_on = ShiftValid::Saturday;
                }
                else if metadata.contains("ZO"){
                    valid_on = ShiftValid::Sunday;
                }
            }
        }
        else if last_y != item.1 .1 {
            // println!("Job gevonden!\nLijn {lijn:?}, omloop {omloop:?}, rit {rit:?}, van {van:?}, naar {naar:?}, begint om {start:?} en stopt om {eind:?}");
            let job = job_creator(lijn, omloop, rit, start, eind, van, naar)?;
            println!("{:?}",&job);
            jobs.push(job);
            lijn = None;
            omloop = None;
            rit = None;
            start = None;
            van = None;
            naar = None;
            eind = None;
        }
        if item.1.0 >= lijn_lower && item.1.0 <= lijn_upper {
            lijn = Some(item.0);
        } else if item.1.0 >= omloop_lower && item.1.0 <= omloop_upper {
            omloop = Some(item.0);
        } else if item.1.0 >= rit_lower && item.1.0 <= rit_upper {
            rit = Some(item.0);
        } else if item.1.0 >= start_lower && item.1.0 <= start_upper {
            start = Some(item.0);
        } else if item.1.0 >= van_lower && item.1.0 <= van_upper {
            van = Some(item.0);
        } else if item.1.0 >= naar_lower && item.1.0 <= naar_upper {
            naar = Some(item.0);
        } else if item.1.0 >= eind_lower {
            eind = Some(item.0);
        }
        last_y = item.1 .1;
    }
    Ok(Shift{
        shift_nr: shift_number.to_string(),
        valid_on: valid_on,
        location: "todo".to_string(),
        shift_type: None,
        job: jobs,
        starting_date: start_date
    })
}

fn job_creator(
    lijn: Option<String>,
    omloop: Option<String>,
    rit: Option<String>,
    start: Option<String>,
    eind: Option<String>,
    van: Option<String>,
    naar: Option<String>,
) -> GenResult<ShiftJob> {
    let mut omloop_number = None;
    let mut job_type= JobType::Unknown;
    let mut rit_number = None;
    let mut start_time: Option<Time> = None;
    let mut end_time = None;
    if let Some(lijn_string) = lijn {
        if lijn_string == "MAT" {
            job_type = JobType::Rijden { drive_type: JobDrivingType::Mat };
        }
        else if lijn_string == "Pauze" {
            job_type = JobType::Pauze;
        } 
        else if let Ok(lijn_parse) = lijn_string.parse::<u32>() {
            job_type = JobType::Rijden { drive_type: JobDrivingType::Lijn(lijn_parse) };
        } else {
            let message = match message_type_finder(lijn_string.clone()){
                Some(message) => message,
                None => JobMessageType::Other(lijn_string)
            };
            job_type = JobType::Melding {message};
        }
    }
    if let Some(rit_string) = rit {
        rit_number = rit_string.parse::<usize>().ok();
    }
    if let Some(start_string) = start {
        let mut time_split = start_string.split(":").into_iter();
        let hour_noniso = time_split.next().unwrap().parse::<u8>()?;
        let minute = time_split.next().unwrap().parse::<u8>()?;
        let hour_iso = match hour_noniso {
            24.. => hour_noniso-24,
            _ => hour_noniso
        };
        start_time = Time::from_hms(hour_iso, minute, 0).ok();
    }
    if let Some(end_string) = eind {
        let mut time_split = end_string.split(":").into_iter();
        let hour_noniso = time_split.next().unwrap().parse::<u8>()?;
        let minute = time_split.next().unwrap().parse::<u8>()?;
        let hour_iso = match hour_noniso {
            24.. => hour_noniso-24,
            _ => hour_noniso
        };
        end_time = Time::from_hms(hour_iso, minute, 0).ok();
    }
    if let Some(omloop_string) = omloop {
        match omloop_string.as_ref() {
            "Onderbreking" => job_type = JobType::Onderbreking,
            "Loop/Reis" => job_type = JobType::LoopReis,
            "Rijklaar maken" => job_type = JobType::RijklaarMaken,
            "Bus stallen/afm" => job_type = JobType::StallenAfmelden,
            "Reserve" => job_type = JobType::Reserve,
            _ => omloop_number = omloop_string.parse::<usize>().ok()
        };
        
    }

    Ok(ShiftJob{
        job_type,
        start: start_time,
        end: end_time,
        start_location: van,
        end_location: naar,
        omloop: omloop_number,
        rit: rit_number
    })
}

fn message_type_finder(lijn_string: String) -> Option<JobMessageType>{
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
