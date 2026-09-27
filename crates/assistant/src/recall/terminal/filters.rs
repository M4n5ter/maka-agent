/*
 * Licensed to the Apache Software Foundation (ASF) under one
 * or more contributor license agreements.  See the NOTICE file
 * distributed with this work for additional information
 * regarding copyright ownership.  The ASF licenses this file
 * to you under the Apache License, Version 2.0 (the
 * "License"); you may not use this file except in compliance
 * with the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing,
 * software distributed under the License is distributed on an
 * "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
 * KIND, either express or implied.  See the License for the
 * specific language governing permissions and limitations
 * under the License.
 */

use super::*;
use serde::{Deserialize, Serialize};
use view::Field;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct Filters {
    pub query: String,
    pub since: String,
    pub until: String,
    pub session: Option<Selected>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Selected {
    pub id: String,
    pub title: String,
}
impl Filters {
    pub fn read(route: &Value) -> Result<Self, Error> {
        if route.is_null() {
            return Ok(Self::default());
        }
        serde_json::from_value(route.clone()).map_err(|error| Error::Invalid(error.to_string()))
    }
    pub fn validate(&self) -> Result<(), String> {
        let since = day(&self.since, false)?;
        let until = day(&self.until, true)?;
        if self.query.len() > 512
            || self.query.split_whitespace().count() > 8
            || since.zip(until).is_some_and(|(since, until)| since > until)
            || self.session.as_ref().is_some_and(|session| {
                session.id.is_empty() || session.id.len() > 256 || session.title.len() > 512
            })
        {
            return Err("Use up to eight words and UTC dates in YYYY-MM-DD order; From must not follow Through.".into());
        }
        Ok(())
    }
    pub fn request(&self, caller_session: Option<&str>) -> Result<Value, Error> {
        self.validate().map_err(Error::Invalid)?;
        Ok(json!({"terms":terms(&self.query),"limit":20,
            "session_id":caller_session.or(self.session.as_ref().map(|session| session.id.as_str())),
            "since":day(&self.since,false).map_err(Error::Invalid)?,"until":day(&self.until,true).map_err(Error::Invalid)?}))
    }
    pub fn fields(&self) -> Vec<Field> {
        vec![
            line("query", view::build::clean(&self.query, false), 512),
            line("since", &self.since, 10),
            line("until", &self.until, 10),
        ]
    }
    pub fn controls(&self, words: &Words) -> Node {
        stack(
            "filters",
            vec![
                row(
                    "session",
                    vec![
                        text(
                            "scope",
                            self.session
                                .as_ref()
                                .map(|session| view::build::clean(&session.title, false))
                                .unwrap_or_else(|| {
                                    words.t("All conversations", "全部对话", "全部對話")
                                }),
                            Tone::Subtle,
                        ),
                        button("choose-session", "choose-session", Role::Normal),
                    ],
                ),
                row(
                    "time",
                    vec![
                        input(
                            "since",
                            "since",
                            words.t("From (UTC date)", "起始日期（UTC）", "起始日期（UTC）"),
                        ),
                        input(
                            "until",
                            "until",
                            words.t("Through (UTC date)", "结束日期（UTC）", "結束日期（UTC）"),
                        ),
                    ],
                ),
                text(
                    "date-hint",
                    words.t(
                        "YYYY-MM-DD; leave blank for no date bound.",
                        "使用 YYYY-MM-DD；留空表示不限制日期。",
                        "使用 YYYY-MM-DD；留空表示不限制日期。",
                    ),
                    Tone::Subtle,
                ),
            ],
        )
    }
}
/// Calendar-day bounds are inclusive UTC dates, matching history milliseconds.
fn day(value: &str, end: bool) -> Result<Option<u64>, String> {
    if value.is_empty() {
        return Ok(None);
    }
    let invalid = || "Use a valid UTC date in YYYY-MM-DD format (1970–9999).".to_owned();
    if value.len() != 10 || value.as_bytes()[4] != b'-' || value.as_bytes()[7] != b'-' {
        return Err(invalid());
    }
    if !value.bytes().enumerate().all(|(index, byte)| {
        if matches!(index, 4 | 7) {
            byte == b'-'
        } else {
            byte.is_ascii_digit()
        }
    }) {
        return Err(invalid());
    }
    let parts: Vec<_> = value
        .split('-')
        .map(str::parse::<u64>)
        .collect::<Result<_, _>>()
        .map_err(|_| invalid())?;
    if parts.len() != 3 {
        return Err(invalid());
    }
    let (year, month, date) = (parts[0], parts[1], parts[2]);
    if !(1970..=9999).contains(&year) || !(1..=12).contains(&month) {
        return Err(invalid());
    }
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let months = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if date == 0 || date > months[(month - 1) as usize] {
        return Err(invalid());
    }
    let before = |year: u64| year * 365 + year / 4 - year / 100 + year / 400;
    let days =
        before(year - 1) - before(1969) + months[..(month - 1) as usize].iter().sum::<u64>() + date
            - 1;
    Ok(Some(days * 86_400_000 + if end { 86_399_999 } else { 0 }))
}
mod sessions;
pub(super) use sessions::sessions;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utc_dates_cover_exact_days_and_filters_reach_the_query() {
        assert_eq!(day("1970-01-01", false).unwrap(), Some(0));
        assert_eq!(day("1970-01-01", true).unwrap(), Some(86_399_999));
        assert_eq!(day("2000-03-01", false).unwrap(), Some(951_868_800_000));
        assert!(day("2100-02-29", false).is_err());
        assert!(day("2024-02-29", false).is_ok());
        let filters = Filters {
            query: "kernel".into(),
            since: "2024-01-01".into(),
            until: "2024-01-02".into(),
            session: Some(Selected {
                id: "exact-session".into(),
                title: "Session".into(),
            }),
        };
        let request = filters.request(None).unwrap();
        assert_eq!(request["session_id"], "exact-session");
        assert_eq!(request["since"], 1_704_067_200_000_u64);
        assert_eq!(request["until"], 1_704_239_999_999_u64);
        assert_eq!(
            filters.request(Some("caller")).unwrap()["session_id"],
            "caller"
        );
    }
}
