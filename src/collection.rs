use std::{
    collections::HashMap,
    fs,
    sync::{LazyLock, RwLock},
};

use serde::{Deserialize, Serialize};
use time::Date;

use crate::GenResult;

static ALL_TIMETABLE_COLLECTIONS: LazyLock<RwLock<Vec<PdfTimetableCollection>>> =
    LazyLock::new(|| RwLock::new(vec![]));

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ShiftData {
    pub pages: Vec<u32>,
    pub file_id: usize,
    pub shift_prefix: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct PdfTimetableCollection {
    pub valid_from: Date,
    pub files: HashMap<usize, String>,
    pub pages: HashMap<String, ShiftData>,
}

impl PdfTimetableCollection {
    pub fn new() -> Self {
        Self {
            valid_from: Date::from_iso_week_date(2000, 20, time::Weekday::Monday).unwrap(),
            files: HashMap::new(),
            pages: HashMap::new(),
        }
    }

    pub fn load_timetables_from_disk() -> GenResult<()> {
        let collections_on_disk = fs::read_dir("pdf_collection")?;
        let mut collections: Vec<Self> = vec![];
        for file_result in collections_on_disk {
            let file = file_result?;
            if file.file_type()?.is_dir() {
                continue;
            }
            let collection_file: PdfTimetableCollection =
                serde_json::from_slice(&fs::read(file.path())?)?;
            collections.push(collection_file);
        }
        collections.sort_by_key(|key| key.valid_from);
        *ALL_TIMETABLE_COLLECTIONS.try_write()? = collections;
        Ok(())
    }

    pub fn get_timetables() -> GenResult<Vec<Self>> {
        let collections = (*ALL_TIMETABLE_COLLECTIONS.try_read()?).to_vec();
        Ok(collections)
    }
}
