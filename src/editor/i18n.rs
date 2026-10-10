//! UI strings from the original app's lang/*.json files (HTML markup stripped).

use std::collections::HashMap;

pub const LANGS: [&str; 7] = ["en", "es", "de", "fr", "ko", "ja", "zh_CN"];

fn source(lang: &str) -> &'static str {
    match lang {
        "es" => include_str!("../../assets/lang/es.json"),
        "de" => include_str!("../../assets/lang/de.json"),
        "fr" => include_str!("../../assets/lang/fr.json"),
        "ko" => include_str!("../../assets/lang/ko.json"),
        "ja" => include_str!("../../assets/lang/ja.json"),
        "zh_CN" => include_str!("../../assets/lang/zh_CN.json"),
        _ => include_str!("../../assets/lang/en.json"),
    }
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse(lang: &str) -> HashMap<String, String> {
    serde_json::from_str::<HashMap<String, String>>(source(lang))
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k, strip_tags(&v)))
        .collect()
}

pub struct Tr {
    map: HashMap<String, String>,
    fallback: HashMap<String, String>,
}

impl Tr {
    pub fn load(lang: &str) -> Self {
        Self { map: parse(lang), fallback: parse("en") }
    }

    pub fn t<'a>(&'a self, key: &'a str) -> &'a str {
        self.map.get(key).or_else(|| self.fallback.get(key)).map_or(key, String::as_str)
    }
}
