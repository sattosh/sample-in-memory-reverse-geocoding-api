use serde_json::json;
use serde_json::Value;
use shapefile::dbase::FieldValue;
use shapefile::dbase::Record;

// 仮の FieldValue の変換関数
pub fn field_value_to_json(value: &FieldValue) -> Value {
    match value {
        FieldValue::Character(opt) => {
            if let Some(s) = opt {
                json!(s)
            } else {
                Value::Null
            }
        }
        // 他の型の場合もここに実装する
        _ => Value::Null,
    }
}

// record から JSON オブジェクトを作成する例
pub fn record_to_json(record: &Record) -> Value {
    let mut map = serde_json::Map::new();
    for (_, v) in record.clone().into_iter().enumerate() {
        map.insert(v.0.to_string(), field_value_to_json(&v.1));
    }
    Value::Object(map)
}
