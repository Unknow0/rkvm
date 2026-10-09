use serde::{Deserialize, Serializer, Deserializer};
use std::str::FromStr;
use tokio::time::Duration;

pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    {
    let nanos = duration.as_nanos();
    let value = if nanos % 1_000_000_000 == 0 {
        format!("{}s", nanos / 1_000_000_000)
    } else if nanos % 1_000_000 == 0 {
        format!("{}ms", nanos / 1_000_000)
    } else if nanos % 1_000 == 0 {
        format!("{}µs", nanos / 1_000)
    } else {
        format!("{}ns", nanos)
    };

    serializer.serialize_str(&value)
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
    {
    let value = String::deserialize(deserializer)?;
    if let Some(v) = value.strip_suffix("ms") {
        let number = u64::from_str(v).map_err(|_| serde::de::Error::custom("invalid duration value"))?;
        Ok(Duration::from_millis(number))
    } else if let Some(v) = value.strip_suffix("µs") {
        let number = u64::from_str(v).map_err(|_| serde::de::Error::custom("invalid duration value"))?;
        Ok(Duration::from_micros(number))
    } else if let Some(v) = value.strip_suffix("ns") {
        let number = u64::from_str(v).map_err(|_| serde::de::Error::custom("invalid duration value"))?;
        Ok(Duration::from_nanos(number))
    } else if let Some(v) = value.strip_suffix('s') {
        let number = u64::from_str(v).map_err(|_| serde::de::Error::custom("invalid duration value"))?;
        Ok(Duration::from_secs(number))
    } else {
        Err(serde::de::Error::custom("invalid duration unit, expected s, ms, µs or ns"))
    }
}