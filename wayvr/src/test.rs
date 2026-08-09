#[cfg(test)]
mod tests {
    use crate::gui::asset::GuiAsset;

    #[test]
    fn lang_json_files_are_valid() {
        for file in GuiAsset::iter() {
            if file.ends_with(".json") {
                let data = GuiAsset::get(&file)
                    .unwrap_or_else(|| panic!("embedded file {file} not found"));
                let text = String::from_utf8(data.data.to_vec())
                    .unwrap_or_else(|e| panic!("file {file} is not valid UTF-8: {e}"));
                serde_json::from_str::<serde_json::Value>(&text)
                    .unwrap_or_else(|e| panic!("file {file} is not valid JSON: {e}"));
            }
        }
    }

    fn parse_csv_keys(data: &[u8]) -> Vec<String> {
        let text = std::str::from_utf8(data).expect("en.csv is not valid UTF-8");
        let mut keys = Vec::new();
        let mut chars = text.chars().peekable();
        while chars.peek().is_some() {
            let key = read_csv_field(&mut chars);
            // skip comma separator
            if chars.peek() == Some(&',') {
                chars.next();
            }
            // skip value field
            read_csv_field(&mut chars);
            // skip comma separator
            if chars.peek() == Some(&',') {
                chars.next();
            }
            // skip description field (may contain newlines via quotes)
            read_csv_field(&mut chars);
            // skip newline
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            keys.push(key);
        }
        keys
    }

    fn read_csv_field(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
        let mut field = String::new();
        if chars.peek() == Some(&'"') {
            // quoted field
            chars.next(); // consume opening quote
            loop {
                match chars.next() {
                    Some('"') => {
                        if chars.peek() == Some(&'"') {
                            field.push('"'); // escaped quote
                            chars.next();
                        } else {
                            break; // closing quote
                        }
                    }
                    Some(c) => field.push(c),
                    None => break,
                }
            }
        } else {
            // unquoted field
            for c in chars.by_ref() {
                if c == ',' || c == '\n' {
                    break;
                }
                field.push(c);
            }
        }
        field
    }

    fn get_nested<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
        let mut current = value;
        for segment in key.split('.') {
            current = current.get(segment)?;
        }
        Some(current)
    }

    #[test]
    fn lang_json_files_contain_all_keys() {
        let en_data = GuiAsset::get("lang/en.csv").expect("en.csv not found");
        let expected_keys = parse_csv_keys(&en_data.data);

        for file in GuiAsset::iter() {
            if !file.ends_with(".json") {
                continue;
            }
            let data =
                GuiAsset::get(&file).unwrap_or_else(|| panic!("embedded file {file} not found"));
            let text = String::from_utf8(data.data.to_vec())
                .unwrap_or_else(|e| panic!("file {file} is not valid UTF-8: {e}"));
            let json: serde_json::Value = serde_json::from_str(&text)
                .unwrap_or_else(|e| panic!("file {file} is not valid JSON: {e}"));

            for key in &expected_keys {
                assert!(
                    get_nested(&json, key).is_some(),
                    "missing key {key} in {file}"
                );
            }
        }
    }
}
