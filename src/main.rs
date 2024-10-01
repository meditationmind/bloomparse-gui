#![allow(dead_code)]
#![windows_subsystem = "windows"]

extern crate tinyfiledialogs;

use chrono::{self, Utc};
use quick_xml::events::attributes::AttrError;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use tinyfiledialogs::{MessageBoxIcon, YesNo};

#[derive(Debug)]
enum AppError {
    /// XML parsing error
    Xml(quick_xml::Error),
    /// Not a MindfulSession record
    NoRecord(String),
}

impl From<quick_xml::Error> for AppError {
    fn from(error: quick_xml::Error) -> Self {
        Self::Xml(error)
    }
}

impl From<AttrError> for AppError {
    fn from(error: AttrError) -> Self {
        Self::Xml(quick_xml::Error::InvalidAttr(error))
    }
}

#[derive(Debug, PartialEq, Deserialize)]
struct MindfulSession {
    #[serde(rename = "@sourceName")]
    pub app: String,
    #[serde(rename = "@startDate")]
    pub start: String,
    #[serde(rename = "@endDate")]
    pub end: String,
}

impl MindfulSession {
    async fn new_from_element(
        reader: &mut Reader<std::io::BufReader<std::fs::File>>,
        element: BytesStart<'_>,
    ) -> Result<Option<MindfulSession>, quick_xml::Error> {
        let mut activity = Cow::Borrowed("");
        let mut app = Cow::Borrowed("");
        let mut start = Cow::Borrowed("");
        let mut end = Cow::Borrowed("");

        for attr_result in element.attributes() {
            let a = attr_result?;
            match a.key.as_ref() {
                b"type" => activity = a.decode_and_unescape_value(reader.decoder())?,
                b"sourceName" => app = a.decode_and_unescape_value(reader.decoder())?,
                b"startDate" => start = a.decode_and_unescape_value(reader.decoder())?,
                b"endDate" => end = a.decode_and_unescape_value(reader.decoder())?,
                _ => (),
            }
        }

        if activity != "HKCategoryTypeIdentifierMindfulSession" {
            return Ok(None);
        }

        Ok(Some(MindfulSession {
            app: app.into(),
            start: start.into(),
            end: end.into(),
        }))
    }
}

#[derive(Debug, Serialize)]
struct BloomRecord {
    #[serde(rename = "App Name")]
    app_name: String,
    #[serde(rename = "Start Time")]
    occurred_at: chrono::DateTime<Utc>,
    #[serde(rename = "Duration")]
    meditation_minutes: i32,
    #[serde(rename = "Dropped Seconds")]
    dropped_seconds: i32,
}

impl BloomRecord {
    async fn new_from_user_data(
        user_record: MindfulSession,
    ) -> Result<BloomRecord, std::num::TryFromIntError> {
        let app_name = user_record.app;
        let occurred_at =
            chrono::NaiveDateTime::parse_from_str(&user_record.start, "%Y-%m-%d %H:%M:%S %z")
                .unwrap()
                .and_utc();
        let end_time =
            chrono::NaiveDateTime::parse_from_str(&user_record.end, "%Y-%m-%d %H:%M:%S %z")
                .unwrap()
                .and_utc();
        //let meditation_minutes: i32 = (end_time - occurred_at).num_minutes().try_into()?;
        let num_seconds: i32 = (end_time - occurred_at).num_seconds().try_into()?;
        let meditation_minutes = num_seconds / 60;
        let dropped_seconds = num_seconds % 60;

        Ok(BloomRecord {
            app_name,
            occurred_at,
            meditation_minutes,
            dropped_seconds,
        })
    }

    async fn write_csv(bloom_data: &Vec<BloomRecord>) -> Result<String, csv::Error> {
        let output_file =
            tinyfiledialogs::save_file_dialog("Save Mindful Session CSV", "bloom-data-ah.csv")
                .map(String::from);

        if output_file.is_none() {
            return Ok("abort".to_owned());
        }

        let filename = output_file.unwrap();
        let mut wtr = csv::WriterBuilder::new().from_path(&filename)?;
        for record in bloom_data {
            if record.meditation_minutes == 0 {
                continue;
            }
            wtr.serialize(record)?;
        }
        wtr.flush()?;

        Ok(filename)
    }

    async fn calculate_stats(bloom_data: Vec<BloomRecord>) -> Result<String, std::io::Error> {
        let mut stats = String::new();
        let mut stats_hash: HashMap<String, i32> = HashMap::new();
        for record in &bloom_data {
            if let Some(value) = stats_hash.get_mut(&record.app_name) {
                *value += 1;
            } else {
                stats_hash.insert(record.app_name.clone(), 1);
            }
        }

        let mut stats_sorted: Vec<(&String, &i32)> = stats_hash.iter().collect();
        stats_sorted.sort_by(|a, b| a.1.cmp(b.1));

        //for key in stats_hash.keys() {
        for (app, total) in stats_sorted {
            //let _ = writeln!(stats, "{key}: {} entries", stats_hash[key]);
            let _ = writeln!(
                stats,
                "{}: {} {}",
                app,
                total,
                if *total == 1 { "entry" } else { "entries" }
            );
        }

        Ok(stats)
    }
}

async fn apple_health(file: &PathBuf) -> Result<(), quick_xml::DeError> {
    let mut reader = Reader::from_file(file)?;

    let mut user_data: Vec<MindfulSession> = Vec::new();
    let mut bloom_data: Vec<BloomRecord> = Vec::new();

    let mut buf = Vec::new();

    loop {
        let event = reader.read_event_into(&mut buf)?;

        match event {
            Event::Empty(element) => {
                if element.name().as_ref() == b"Record" {
                    if let Some(entry) = MindfulSession::new_from_element(&mut reader, element)
                        .await
                        .unwrap()
                    {
                        user_data.push(entry);
                    } else {
                        continue;
                    }
                }
            }
            Event::Eof => break,
            _ => (),
        }
    }

    for record in user_data {
        bloom_data.push(BloomRecord::new_from_user_data(record).await.unwrap());
    }

    if bloom_data.len().eq(&0) {
        tinyfiledialogs::message_box_ok(
            "Bloom Bot Parser",
            "No Mindful Session entries found.",
            MessageBoxIcon::Warning,
        );
        return Ok(());
    }

    //let mut map: HashMap<&str, Vec<(chrono::DateTime<Utc>, i32)>> = HashMap::new();
    //for record in &bloom_data {
    //    if let Some(key) = map.get_mut(record.app_name.as_str()) { key.push((record.occurred_at, record.meditation_minutes)) }
    //    else { map.insert(record.app_name.as_str(), vec![(record.occurred_at, record.meditation_minutes)]); }
    //}

    let filename = BloomRecord::write_csv(&bloom_data).await.unwrap();
    let stats = BloomRecord::calculate_stats(bloom_data).await.unwrap();

    if filename == "abort" {
        tinyfiledialogs::message_box_ok(
            "Bloom Bot Parser",
            "Mindful Session extraction cancelled.",
            MessageBoxIcon::Warning,
        );
        return Ok(());
    }

    tinyfiledialogs::message_box_ok(
        "Bloom Bot Parser",
        format!(
            "Mindful Session extraction successful!\n\n{}\nUpload {} to the #meditation-tracking channel and use /import to import the data into Bloom.",
            stats,
            filename.split("\\").last().unwrap()
        ).as_str(),
        MessageBoxIcon::Info,
    );

    Ok(())
}

#[tokio::main]
async fn main() {
    let proceed = tinyfiledialogs::message_box_yes_no(
        "Bloom Bot Parser",
        "This will extract all Mindful Sessions from your Apple Health data into a CSV file, which can be imported using Bloom. Proceed?",
        MessageBoxIcon::Question,
        YesNo::Yes,
    );

    if let YesNo::No = proceed {
        tinyfiledialogs::message_box_ok(
            "Bloom Bot Parser",
            "Mindful Session extraction cancelled.",
            MessageBoxIcon::Warning,
        );
        return;
    }

    let input_file = tinyfiledialogs::open_file_dialog(
        "Open Apple Health data",
        "/export.xml",
        Some((&["*.xml"], "Apple Health export data (*.xml)")),
    )
    .map(PathBuf::from);

    if input_file.is_none() {
        tinyfiledialogs::message_box_ok(
            "Bloom Bot Parser",
            "Mindful Session extraction cancelled.",
            MessageBoxIcon::Warning,
        );
        return;
    }

    if let Err(err) = apple_health(&input_file.unwrap()).await {
        tinyfiledialogs::message_box_ok(
            "Bloom Bot Parser",
            format!("Error extracting Mindful Sessions: {}", err).as_str(),
            MessageBoxIcon::Error,
        );
    }
}
