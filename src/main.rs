#![warn(clippy::pedantic, clippy::unwrap_used)]
#![windows_subsystem = "windows"]

extern crate tinyfiledialogs;

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs::File;
use std::io::BufReader;
use std::num::TryFromIntError;
use std::path::PathBuf;

use chrono::{self, DateTime, NaiveDateTime, Utc};
use csv::{Error as CsvError, WriterBuilder};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{DeError, Error as QuickXmlError, Reader, XmlVersion};
use serde::{Deserialize, Serialize};
use tinyfiledialogs::{MessageBoxIcon, YesNo};

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
    fn new_from_element(
        reader: &mut Reader<BufReader<File>>,
        element: &BytesStart<'_>,
    ) -> Result<Option<MindfulSession>, QuickXmlError> {
        let version = XmlVersion::default();
        let decoder = reader.decoder();

        let mut app = Cow::Borrowed("");
        let mut start = Cow::Borrowed("");
        let mut end = Cow::Borrowed("");

        for a in element.attributes().flatten() {
            match a.key.as_ref() {
                b"type"
                    if a.decoded_and_normalized_value(version, decoder)?
                        != "HKCategoryTypeIdentifierMindfulSession" =>
                {
                    return Ok(None);
                }
                b"sourceName" => app = a.decoded_and_normalized_value(version, decoder)?,
                b"startDate" => start = a.decoded_and_normalized_value(version, decoder)?,
                b"endDate" => end = a.decoded_and_normalized_value(version, decoder)?,
                _ => {}
            }
        }

        Ok(Some(MindfulSession {
            app: app.into_owned(),
            start: start.into_owned(),
            end: end.into_owned(),
        }))
    }
}

#[derive(Debug, Serialize)]
struct BloomRecord {
    #[serde(rename = "App Name")]
    app_name: String,
    #[serde(rename = "Start Time")]
    occurred_at: DateTime<Utc>,
    #[serde(rename = "Minutes")]
    meditation_minutes: i32,
    #[serde(rename = "Seconds")]
    meditation_seconds: i32,
}

impl BloomRecord {
    fn new_from_user_data(user_record: MindfulSession) -> Result<BloomRecord, TryFromIntError> {
        let app_name = user_record.app;
        let occurred_at = NaiveDateTime::parse_from_str(&user_record.start, "%Y-%m-%d %H:%M:%S %z")
            .unwrap_or_default()
            .and_utc();
        let end_time = NaiveDateTime::parse_from_str(&user_record.end, "%Y-%m-%d %H:%M:%S %z")
            .unwrap_or_default()
            .and_utc();
        let num_seconds: i32 = (end_time - occurred_at).num_seconds().try_into()?;
        let meditation_minutes = num_seconds / 60;
        let meditation_seconds = num_seconds % 60;

        Ok(BloomRecord {
            app_name,
            occurred_at,
            meditation_minutes,
            meditation_seconds,
        })
    }

    fn write_csv(bloom_data: &Vec<BloomRecord>) -> Result<String, CsvError> {
        let output_file =
            tinyfiledialogs::save_file_dialog("Save Mindful Session CSV", "bloom-data-ah.csv");

        let Some(filename) = output_file else {
            return Ok("abort".to_owned());
        };

        let mut wtr = WriterBuilder::new().from_path(&filename)?;
        for record in bloom_data {
            if record.meditation_minutes == 0 && record.meditation_seconds == 0 {
                continue;
            }
            wtr.serialize(record)?;
        }
        wtr.flush()?;

        Ok(filename)
    }

    fn calculate_stats(bloom_data: &[BloomRecord]) -> String {
        let mut stats = String::new();
        let mut stats_hash: HashMap<String, i32> = HashMap::new();
        for record in bloom_data {
            if let Some(value) = stats_hash.get_mut(&record.app_name) {
                *value += 1;
            } else {
                stats_hash.insert(record.app_name.clone(), 1);
            }
        }

        let mut stats_sorted: Vec<(&String, &i32)> = stats_hash.iter().collect();
        stats_sorted.sort_by(|a, b| a.1.cmp(b.1));

        for (app, total) in stats_sorted {
            let _ = writeln!(
                stats,
                "{}: {} {}",
                app,
                total,
                if *total == 1 { "entry" } else { "entries" }
            );
        }

        stats
    }
}

fn apple_health(file: &PathBuf) -> Result<(), DeError> {
    let mut reader = Reader::from_file(file)?;

    let mut user_data: Vec<MindfulSession> = Vec::new();
    let mut bloom_data: Vec<BloomRecord> = Vec::new();

    let mut buf = Vec::new();

    loop {
        let event = reader.read_event_into(&mut buf)?;

        match event {
            Event::Empty(element) => {
                if element.name().as_ref() == b"Record"
                    && let Some(entry) =
                        MindfulSession::new_from_element(&mut reader, &element).unwrap_or(None)
                {
                    user_data.push(entry);
                }
            }
            Event::Eof => break,
            _ => (),
        }
    }

    for record in user_data {
        let Ok(processed_record) = BloomRecord::new_from_user_data(record) else {
            continue;
        };
        if processed_record.occurred_at == DateTime::UNIX_EPOCH {
            continue;
        }
        bloom_data.push(processed_record);
    }

    if bloom_data.len().eq(&0) {
        tinyfiledialogs::message_box_ok(
            "Bloom Bot Parser",
            "No Mindful Session entries found.",
            MessageBoxIcon::Warning,
        );
        return Ok(());
    }

    let Ok(filename) = BloomRecord::write_csv(&bloom_data) else {
        tinyfiledialogs::message_box_ok(
            "Bloom Bot Parser",
            "Mindful Session extraction failed. Please try again or contact server staff for assistance.",
            MessageBoxIcon::Warning,
        );
        return Ok(());
    };
    let stats = BloomRecord::calculate_stats(&bloom_data);

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
            "Mindful Session extraction successful!\n\n{stats}\nUpload {} to the #meditation-tracking channel and use /import to import the data into Bloom.",
            filename.split('\\').next_back().unwrap_or("the CSV file")
        ).as_str(),
        MessageBoxIcon::Info,
    );

    Ok(())
}

fn main() {
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

    let Some(input_file) = tinyfiledialogs::open_file_dialog(
        "Open Apple Health data",
        "/export.xml",
        Some((&["*.xml"], "Apple Health export data (*.xml)")),
    )
    .map(PathBuf::from) else {
        tinyfiledialogs::message_box_ok(
            "Bloom Bot Parser",
            "Mindful Session extraction cancelled.",
            MessageBoxIcon::Warning,
        );
        return;
    };

    if let Err(err) = apple_health(&input_file) {
        tinyfiledialogs::message_box_ok(
            "Bloom Bot Parser",
            format!("Error extracting Mindful Sessions: {err}").as_str(),
            MessageBoxIcon::Error,
        );
    }
}
