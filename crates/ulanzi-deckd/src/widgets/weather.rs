//! Weather from Open-Meteo (no key): look up the town once, then the forecast every 15 minutes.

use ulanzi_core::render::screen::WeatherData;

pub const REFRESH_SECS: u64 = 15 * 60;

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".into(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn geocode_url(place: &str) -> String {
    format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=1&language=en&format=json",
        encode(place.trim())
    )
}

pub fn forecast_url(lat: f64, lon: f64, fahrenheit: bool) -> String {
    format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}\
         &current=temperature_2m,weather_code&daily=temperature_2m_max,temperature_2m_min\
         &timezone=auto&forecast_days=1{}",
        if fahrenheit {
            "&temperature_unit=fahrenheit"
        } else {
            ""
        }
    )
}

/// (latitude, longitude, display name) of the first match.
pub fn parse_geocode(json: &str) -> Option<(f64, f64, String)> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let r = v.get("results")?.get(0)?;
    Some((
        r.get("latitude")?.as_f64()?,
        r.get("longitude")?.as_f64()?,
        r.get("name")?.as_str()?.to_string(),
    ))
}

pub fn parse_forecast(json: &str, place: &str) -> Result<WeatherData, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|_| "unexpected reply".to_string())?;
    let num = |p: &str| v.pointer(p).and_then(|x| x.as_f64());
    Ok(WeatherData::Ok {
        temp: num("/current/temperature_2m").ok_or("no temperature")? as f32,
        code: num("/current/weather_code").unwrap_or(3.0) as u8,
        place: place.to_string(),
        high: num("/daily/temperature_2m_max/0").unwrap_or(f64::NAN) as f32,
        low: num("/daily/temperature_2m_min/0").unwrap_or(f64::NAN) as f32,
    })
}

fn get(url: &str) -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(8)))
        .build()
        .into();
    agent
        .get(url)
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(c) => format!("the weather service said {c}"),
            _ => "no connection".to_string(),
        })?
        .body_mut()
        .with_config()
        .limit(1024 * 1024)
        .read_to_string()
        .map_err(|_| "unreadable reply".to_string())
}

/// A town found by name.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub name: String,
    pub region: String,
    pub country: String,
    pub lat: f64,
    pub lon: f64,
}

impl Place {
    /// "Town, Region, Country", leaving out parts that repeat the town.
    pub fn label(&self) -> String {
        let mut parts = vec![self.name.clone()];
        for p in [&self.region, &self.country] {
            if !p.is_empty() && !parts.contains(p) {
                parts.push(p.clone());
            }
        }
        parts.join(", ")
    }
}

/// The town part of a label ("Portland, Oregon, …" → "Portland"), for the widget.
pub fn short_name(label: &str) -> &str {
    label.split(',').next().unwrap_or(label).trim()
}

pub fn places_url(query: &str) -> String {
    format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&count=8&language=en&format=json",
        encode(query.trim())
    )
}

pub fn parse_places(json: &str) -> Vec<Place> {
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or_default();
    let s =
        |r: &serde_json::Value, k: &str| r.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
    v.get("results")
        .and_then(|r| r.as_array())
        .into_iter()
        .flatten()
        .filter_map(|r| {
            Some(Place {
                name: s(r, "name"),
                region: s(r, "admin1"),
                country: s(r, "country"),
                lat: r.get("latitude")?.as_f64()?,
                lon: r.get("longitude")?.as_f64()?,
            })
        })
        .collect()
}

/// Towns matching `query` (blocking).
pub fn search(query: &str) -> Result<Vec<Place>, String> {
    if query.trim().chars().count() < 2 {
        return Ok(Vec::new());
    }
    get(&places_url(query)).map(|j| parse_places(&j))
}

/// The weather at known coordinates (blocking); `place` is the label shown.
pub fn fetch_at(place: &str, lat: f64, lon: f64, fahrenheit: bool) -> WeatherData {
    get(&forecast_url(lat, lon, fahrenheit))
        .and_then(|j| parse_forecast(&j, short_name(place)))
        .unwrap_or_else(WeatherData::Err)
}

/// Look up `place` and fetch its weather (blocking; run it off the deck's task).
pub fn fetch(place: &str, fahrenheit: bool) -> WeatherData {
    let found = get(&geocode_url(place))
        .and_then(|j| parse_geocode(&j).ok_or_else(|| format!("can't find “{place}”")));
    let result = found.and_then(|(lat, lon, name)| {
        get(&forecast_url(lat, lon, fahrenheit)).and_then(|j| parse_forecast(&j, &name))
    });
    result.unwrap_or_else(WeatherData::Err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weather_parsing() {
        let geo =
            r#"{"results":[{"name":"Singapore","latitude":1.29,"longitude":103.85,"country":"Singapore"}]}"#;
        assert_eq!(parse_geocode(geo), Some((1.29, 103.85, "Singapore".to_string())));
        assert_eq!(parse_geocode(r#"{"generationtime_ms":0.5}"#), None);
        let fc = r#"{"current":{"temperature_2m":31.4,"weather_code":2},"daily":{"temperature_2m_max":[33.0],"temperature_2m_min":[26.1]}}"#;
        assert_eq!(
            parse_forecast(fc, "Singapore"),
            Ok(WeatherData::Ok {
                temp: 31.4,
                code: 2,
                place: "Singapore".into(),
                high: 33.0,
                low: 26.1
            })
        );
        assert!(geocode_url("São Paulo").contains("S%C3%A3o+Paulo"));
        assert!(forecast_url(1.0, 2.0, true).ends_with("&temperature_unit=fahrenheit"));
    }

    #[test]
    fn places_list_town_region_and_country() {
        let json = r#"{"results":[
            {"name":"Portland","latitude":45.52,"longitude":-122.68,"admin1":"Oregon","country":"United States"},
            {"name":"Portland","latitude":43.66,"longitude":-70.25,"admin1":"Maine","country":"United States"},
            {"name":"Singapore","latitude":1.29,"longitude":103.85,"admin1":"Singapore","country":"Singapore"}]}"#;
        let p = parse_places(json);
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].label(), "Portland, Oregon, United States");
        assert_eq!(p[1].lat, 43.66);
        // No repeats when the region or country is the town itself.
        assert_eq!(p[2].label(), "Singapore");
        assert!(parse_places("{}").is_empty());
        assert!(places_url("New York").contains("count=8"));
        assert_eq!(short_name("Portland, Oregon, United States"), "Portland");
    }

    #[test]
    fn weather_errors_become_text() {
        assert!(parse_forecast("<html>busy</html>", "X").is_err());
        assert!(parse_forecast(r#"{"current":{}}"#, "X").is_err());
    }
}
