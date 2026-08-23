#![warn(clippy::pedantic)]
#![cfg_attr(not(test), windows_subsystem = "windows")]
#![cfg_attr(test, windows_subsystem = "console")]

extern crate tinyfiledialogs;

use std::borrow::Cow;
use std::fmt::Write as _;
use std::num::TryFromIntError;
use std::path::{Path, PathBuf};

use chrono::{self, DateTime, Utc};
use csv::{Error as CsvError, WriterBuilder};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{DeError, Error as QuickXmlError, Reader, XmlVersion};
use rapidhash::RapidHashMap;
use serde::{Deserialize, Serialize};
use tinyfiledialogs::{MessageBoxIcon, YesNo};

#[derive(Debug, PartialEq, Deserialize)]
struct MindfulSession<'a> {
    #[serde(rename = "@sourceName")]
    app: Cow<'a, str>,
    #[serde(rename = "@startDate")]
    start: Cow<'a, str>,
    #[serde(rename = "@endDate")]
    end: Cow<'a, str>,
}

impl<'a> MindfulSession<'_> {
    fn new_from_element(
        element: &'a BytesStart<'_>,
    ) -> Result<Option<MindfulSession<'a>>, QuickXmlError> {
        let version = XmlVersion::default();

        let mut app = Cow::Borrowed("");
        let mut start = Cow::Borrowed("");
        let mut end = Cow::Borrowed("");

        for a in element.attributes().flatten() {
            match a.key.into_inner() {
                "type"
                    if a.normalized_value(version)? != "HKCategoryTypeIdentifierMindfulSession" =>
                {
                    return Ok(None);
                }
                "sourceName" => app = a.normalized_value(version)?,
                "startDate" => start = a.normalized_value(version)?,
                "endDate" => end = a.normalized_value(version)?,
                _ => {}
            }
        }

        Ok(Some(MindfulSession { app, start, end }))
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
        let app_name = user_record.app.into_owned();
        let occurred_at = DateTime::parse_from_str(&user_record.start, "%Y-%m-%d %H:%M:%S %z")
            .unwrap_or_default()
            .to_utc();
        let end_time = DateTime::parse_from_str(&user_record.end, "%Y-%m-%d %H:%M:%S %z")
            .unwrap_or_default()
            .to_utc();
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

    fn write_csv(bloom_data: &Vec<BloomRecord>) -> Result<Option<String>, CsvError> {
        let filename =
            tinyfiledialogs::save_file_dialog("Save Mindful Session CSV", "bloom-data-ah.csv");

        if let Some(filename) = &filename {
            let mut wtr = WriterBuilder::new().from_path(filename)?;
            for record in bloom_data {
                if record.meditation_minutes > 0 || record.meditation_seconds > 0 {
                    wtr.serialize(record)?;
                }
            }
            wtr.flush()?;
        }

        Ok(filename)
    }

    fn calculate_stats(bloom_data: &[BloomRecord]) -> String {
        let mut stats = String::new();
        let mut stats_hash = RapidHashMap::default();

        for record in bloom_data {
            stats_hash
                .entry(record.app_name.as_str())
                .and_modify(|v| *v += 1)
                .or_insert(1);
        }

        let mut stats_sorted: Vec<(&str, i32)> = stats_hash.into_iter().collect();
        stats_sorted.sort_by_key(|a| a.1);
        stats_sorted.reverse();

        for (app, total) in stats_sorted {
            let entry = if total == 1 { "entry" } else { "entries" };
            let _ = writeln!(stats, "{app}: {total} {entry}");
        }

        stats
    }
}

fn apple_health(file: &Path) -> Result<(), DeError> {
    let mut reader = Reader::from_file(file)?;
    let mut bloom_data: Vec<BloomRecord> = Vec::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Empty(element) | Event::Start(element) => {
                if element.name().into_inner() == "Record"
                    && let Some(entry) = MindfulSession::new_from_element(&element).unwrap_or(None)
                    && let Ok(processed_record) = BloomRecord::new_from_user_data(entry)
                    && processed_record.occurred_at != DateTime::UNIX_EPOCH
                {
                    bloom_data.push(processed_record);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    if bloom_data.len().eq(&0) {
        tinyfiledialogs::message_box_ok(
            "Bloom Data Parser",
            "No Mindful Session entries found.",
            MessageBoxIcon::Warning,
        );
        return Ok(());
    }

    let Ok(filename) = BloomRecord::write_csv(&bloom_data) else {
        tinyfiledialogs::message_box_ok(
            "Bloom Data Parser",
            "Mindful Session extraction failed. Please try again or contact server staff for assistance.",
            MessageBoxIcon::Warning,
        );
        return Ok(());
    };

    let Some(filename) = filename else {
        tinyfiledialogs::message_box_ok(
            "Bloom Data Parser",
            "Mindful Session extraction cancelled.",
            MessageBoxIcon::Warning,
        );
        return Ok(());
    };

    let stats = BloomRecord::calculate_stats(&bloom_data);

    tinyfiledialogs::message_box_ok(
        "Bloom Data Parser",
        format!(
            "Mindful Session extraction successful!\n\n{stats}\nUpload {} to the #meditation-tracking channel and use /import to import the data into Bloom.",
            filename.split('\\').next_back().unwrap_or("the CSV file")
        )
        .as_str(),
        MessageBoxIcon::Info,
    );

    Ok(())
}

fn main() {
    let proceed = tinyfiledialogs::message_box_yes_no(
        "Bloom Data Parser",
        "This will extract all Mindful Sessions from your Apple Health data into a CSV file, which can be imported using Bloom. Proceed?",
        MessageBoxIcon::Question,
        YesNo::Yes,
    );

    if let YesNo::No = proceed {
        tinyfiledialogs::message_box_ok(
            "Bloom Data Parser",
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
            "Bloom Data Parser",
            "Mindful Session extraction cancelled.",
            MessageBoxIcon::Warning,
        );
        return;
    };

    if let Err(err) = apple_health(&input_file) {
        tinyfiledialogs::message_box_ok(
            "Bloom Data Parser",
            format!("Error extracting Mindful Sessions: {err}").as_str(),
            MessageBoxIcon::Error,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_event() -> Result<(), DeError> {
        let mut reader = Reader::from_str(
            "<Record type=\"HKCategoryTypeIdentifierMindfulSession\" sourceName=\"Insight Timer\" sourceVersion=\"14.3.2.287\" creationDate=\"2018-10-28 00:44:29 +0900\" startDate=\"2018-10-28 00:33:23 +0900\" endDate=\"2018-10-28 00:44:20 +0900\" value=\"HKCategoryValueNotApplicable\"/>",
        );
        let event = reader.read_event()?;
        if let Event::Empty(element) | Event::Start(element) = event {
            let entry = MindfulSession::new_from_element(&element)?.unwrap();
            assert_eq!(entry.app, "Insight Timer");
            assert_eq!(entry.start, "2018-10-28 00:33:23 +0900");
            assert_eq!(entry.end, "2018-10-28 00:44:20 +0900");

            let record = BloomRecord::new_from_user_data(entry)
                .map_err(|e| DeError::Custom(e.to_string()))?;
            let time = DateTime::from_timestamp_millis(1_540_654_403_000).unwrap();
            assert_eq!(record.app_name, "Insight Timer");
            assert_eq!(record.occurred_at, time);
            assert_eq!(record.meditation_minutes, 10i32);
            assert_eq!(record.meditation_seconds, 57i32);
        } else {
            panic!("failed to read event");
        }
        Ok(())
    }

    #[test]
    fn test_invalid_event() -> Result<(), DeError> {
        let mut reader = Reader::from_str(
            "<Record type=\"HKCategoryTypeIdentifierSleepAnalysis\" sourceName=\"Withings\" sourceVersion=\"4100100\" creationDate=\"2020-07-13 14:29:25 +0900\" startDate=\"2016-10-27 06:17:13 +0900\" endDate=\"2016-10-27 06:41:13 +0900\" value=\"HKCategoryValueSleepAnalysisAsleepUnspecified\">",
        );
        let event = reader.read_event()?;
        if let Event::Empty(element) | Event::Start(element) = event {
            let entry = MindfulSession::new_from_element(&element)?;
            assert!(entry.is_none());
        } else {
            panic!("failed to read event");
        }
        Ok(())
    }
}
