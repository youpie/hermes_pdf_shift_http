use chrono::NaiveTime;
use lopdf::Document;
use crate::GenResult;
use std::path::PathBuf;

enum ShiftValid {
    Weekdagen,
    Zaterdag,
    Zondag
}

enum ShiftType {
    Vroeg,
    Tussen,
    Gebroken{start_break: Option<NaiveTime>, end_break: Option<NaiveTime>}, // If one is none, it means it's half of a broken shift
    Laat
}

enum JobDrivingType{
    Lijn(u32),
    Mat,
}

enum JobMessageType{
    Meenemen{dienstnummers: Vec<u32>},
    Passagieren{dienstnummer: u32, omloop: u32},
    BusOp{lijn: u32},
    NeemBus{bustype: String},
    Other(String),
}

enum JobType {
    Rijden{drive_type: JobDrivingType},
    Pauze{onderbreking: bool},
    OpAfstap,
    RijklaarMaken,
    StallenAfmelden,
    Melding{message: JobMessageType}
}

struct ShiftJob {
    job_type: JobType,
    start: Option<NaiveTime>,
    end: Option<NaiveTime>,
    start_location: Option<String>,
    end_location: Option<String>,    // If none, it's the same as start
    omloop: Option<u32>
}

struct Shift {
    shift_nr: String,
    start_time: chrono::NaiveTime,
    end_time: chrono::NaiveTime,
    valid_on: ShiftValid,
    location: String,
    shift_type: ShiftType,
    job: Vec<ShiftJob>
}

pub fn read_pdf_stream(pdf_path: PathBuf) -> GenResult<()> {
    let doc = Document::load(pdf_path)?;
    let pages = doc.get_pages();
    for (&page_number, &page_id) in pages.iter(){
        let page_dict = doc.get_object(page_id)?.as_dict()?;
        let contents = page_dict.get(b"Contents")?;
        println!("{:#?}",contents);
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
                println!("Page {} stream: {}", page_number, stream_string);
            }
            _ => {
                println!("Unexpected type for Contents on page {}", page_number);
            }
        }
        break;
    }
    Ok(())
}
