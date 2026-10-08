//! Sampling values as the decimal the client wrote.

use serde_json::{Map, Number, Value};

/// Inserts `value` under `key` as its shortest decimal (`0.7`, not the widened
/// `0.699999988079071`). The widened form is a different number to a validator:
/// Anthropic's extended-thinking rule `top_p >= 0.95` rejects `0.95f32 as f64`.
/// Non-finite values are not sent.
pub fn insert_f32(body: &mut Map<String, Value>, key: &str, value: f32) {
    let Some(number) = format!("{value}")
        .parse::<f64>()
        .ok()
        .and_then(Number::from_f64)
    else {
        return;
    };
    body.insert(key.into(), Value::Number(number));
}

#[cfg(test)]
mod tests {
    use super::*;

    // Defeat: `json!(0.95_f32)`, which serializes 0.949999988079071.
    #[test]
    fn sampling_values_keep_their_shortest_decimal() {
        let mut body = Map::new();
        for (key, v) in [("a", 0.7_f32), ("b", 0.95), ("c", 1.0), ("d", 0.0)] {
            insert_f32(&mut body, key, v);
        }
        insert_f32(&mut body, "nan", f32::NAN);
        assert_eq!(
            Value::Object(body),
            serde_json::json!({"a": 0.7, "b": 0.95, "c": 1.0, "d": 0.0})
        );
    }
}
