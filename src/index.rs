use std::{collections::HashMap, fs};

use time::{Date, OffsetDateTime};

use crate::{GenResult, PdfTimetableCollection};

pub fn get_valid_shifts(date: Option<Date>) -> GenResult<Vec<String>>{
    let collections = fs::read_dir("pdf_collection")?;
    let current_date = match date {
        Some(date) => date,
        None => OffsetDateTime::now_utc().date(),
    };
    let mut active_timetables: Vec<PdfTimetableCollection> = vec![];
    let mut available_shifts: HashMap<String, Date> = HashMap::new();
    // Loop over all files in the collection folder
    for file_result in collections {
        let file = file_result?;
        if file.file_type()?.is_dir() {
            continue;
        }
        let temp_current_collection_file: PdfTimetableCollection =
            serde_json::from_slice(&fs::read(file.path())?)?;
        
        // Create a list of all currently valid timetables
        if temp_current_collection_file.valid_from <= current_date {
            active_timetables.push(temp_current_collection_file);
        }
    }
    for timetable in active_timetables {
        for shift in timetable.pages {
            match available_shifts.get_key_value(&shift.0) {
                Some(already_added) => {
                    match already_added.1 > &timetable.valid_from {
                        true => continue,
                        false => {available_shifts.insert(shift.0,timetable.valid_from);}
                    }
                }
                None => {available_shifts.insert(shift.0,timetable.valid_from);}
            }
        }
    }
    Ok(available_shifts.keys().cloned().collect::<Vec<String>>())
}