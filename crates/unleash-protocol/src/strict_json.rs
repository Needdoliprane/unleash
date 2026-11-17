//! # Décodage JSON strict avec rejet des clés dupliquées
//!
//! Responsabilité exclusive : Valider la conformité stricte d'un document JSON
//! en garantissant l'absence de toute clé dupliquée à tous les niveaux d'imbrication,
//! avant ou pendant la désérialisation Serde.

use crate::errors::ProtocolError;
use serde::de::{self, Deserialize, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use std::collections::HashSet;
use std::fmt;

/// Parseur strict de document JSON sous forme de [`serde_json::Value`],
/// rejetant toute clé dupliquée à n'importe quel niveau d'imbrication.
///
/// # Errors
/// Renvoie [`ProtocolError::DuplicateKey`] si une clé apparaît plus d'une fois dans un même objet,
/// ou [`ProtocolError::Json`] si la syntaxe JSON est invalide.
pub fn parse_json_value_strict(json_str: &str) -> Result<serde_json::Value, ProtocolError> {
    let mut deserializer = serde_json::Deserializer::from_str(json_str);
    let value = StrictValueSeed
        .deserialize(&mut deserializer)
        .map_err(map_strict_json_error)?;
    deserializer.end().map_err(ProtocolError::Json)?;
    Ok(value)
}

fn map_strict_json_error(e: serde_json::Error) -> ProtocolError {
    let msg = e.to_string();
    if let Some(rest) = msg.strip_prefix("duplicate key '")
        && let Some(end_quote) = rest.rfind('\'')
    {
        return ProtocolError::DuplicateKey {
            key: rest[..end_quote].to_owned(),
        };
    }
    ProtocolError::Json(e)
}

/// Désérialise une structure typée `T` à partir d'une chaîne JSON,
/// en garantissant d'abord l'absence totale de clé dupliquée, puis en appliquant
/// les règles Serde (notamment `#[serde(deny_unknown_fields)]`).
///
/// # Errors
/// Renvoie [`ProtocolError::DuplicateKey`] si une clé est dupliquée,
/// ou [`ProtocolError::Json`] si le document est syntaxiquement invalide ou ne correspond pas à `T`.
pub fn from_json_str_strict<'a, T: Deserialize<'a>>(json_str: &'a str) -> Result<T, ProtocolError> {
    let value = parse_json_value_strict(json_str)?;
    T::deserialize(value).map_err(ProtocolError::Json)
}

struct StrictValueSeed;

impl<'de> DeserializeSeed<'de> for StrictValueSeed {
    type Value = serde_json::Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictValueVisitor)
    }
}

struct StrictValueVisitor;

impl<'de> Visitor<'de> for StrictValueVisitor {
    type Value = serde_json::Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("toute valeur JSON valide sans clé dupliquée")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E> {
        Ok(serde_json::Value::Bool(v))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E> {
        Ok(serde_json::Value::Number(v.into()))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
        Ok(serde_json::Value::Number(v.into()))
    }

    fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        serde_json::Number::from_f64(v)
            .map(serde_json::Value::Number)
            .ok_or_else(|| {
                de::Error::custom(format!("nombre flottant JSON non représentable : {v}"))
            })
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
        Ok(serde_json::Value::String(v.to_owned()))
    }

    fn visit_string<E>(self, v: String) -> Result<Self::Value, E> {
        Ok(serde_json::Value::String(v))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(serde_json::Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(serde_json::Value::Null)
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut elements = Vec::new();
        while let Some(elem) = seq.next_element_seed(StrictValueSeed)? {
            elements.push(elem);
        }
        Ok(serde_json::Value::Array(elements))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut object = serde_json::Map::new();
        let mut seen_keys = HashSet::new();

        while let Some(key) = map.next_key::<String>()? {
            if !seen_keys.insert(key.clone()) {
                return Err(de::Error::custom(format!("duplicate key '{key}'")));
            }
            let value = map.next_value_seed(StrictValueSeed)?;
            object.insert(key, value);
        }
        Ok(serde_json::Value::Object(object))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::value::F64Deserializer;

    #[test]
    fn visit_f64_rejects_non_finite_floats() {
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let de = F64Deserializer::<serde_json::Error>::new(invalid);
            let res = StrictValueSeed.deserialize(de);
            assert!(
                res.is_err(),
                "visit_f64 doit rejeter {invalid} par une erreur Serde explicite"
            );
        }

        let valid_de = F64Deserializer::<serde_json::Error>::new(2.5);
        let valid_res = StrictValueSeed.deserialize(valid_de);
        assert!(matches!(valid_res, Ok(serde_json::Value::Number(_))));
    }
}
