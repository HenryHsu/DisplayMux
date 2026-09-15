use std::{collections::BTreeMap, sync::OnceLock};

const ENGLISH_SOURCE: &str = include_str!("../../src/locales/en.json");
const TRADITIONAL_CHINESE_SOURCE: &str = include_str!("../../src/locales/zh-TW.json");

static ENGLISH: OnceLock<BTreeMap<String, String>> = OnceLock::new();
static TRADITIONAL_CHINESE: OnceLock<BTreeMap<String, String>> = OnceLock::new();

fn catalog(traditional_chinese: bool) -> &'static BTreeMap<String, String> {
    let (catalog, source) = if traditional_chinese {
        (&TRADITIONAL_CHINESE, TRADITIONAL_CHINESE_SOURCE)
    } else {
        (&ENGLISH, ENGLISH_SOURCE)
    };

    catalog.get_or_init(|| {
        serde_json::from_str(source).expect("embedded locale resource must contain valid JSON")
    })
}

pub fn get(traditional_chinese: bool, key: &str) -> String {
    catalog(traditional_chinese)
        .get(key)
        .or_else(|| catalog(false).get(key))
        .cloned()
        .unwrap_or_else(|| key.to_owned())
}

pub fn render(template: &str, parameters: &[(&str, &str)]) -> String {
    parameters
        .iter()
        .fold(template.to_owned(), |message, (name, value)| {
            message.replace(&format!("{{{name}}}"), value)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_named_message_parameters() {
        let message = render(
            &get(false, "backend.peerDisplayWakeFailed"),
            &[("peer", "Studio Mac"), ("error", "wake failed")],
        );

        assert!(message.contains("Studio Mac"));
        assert!(message.contains("wake failed"));
        assert!(!message.contains("{peer}"));
        assert!(!message.contains("{error}"));
    }

    #[test]
    fn locale_resources_have_matching_keys() {
        let english = catalog(false);
        let traditional_chinese = catalog(true);

        assert_eq!(
            english.keys().collect::<Vec<_>>(),
            traditional_chinese.keys().collect::<Vec<_>>()
        );
    }
}
