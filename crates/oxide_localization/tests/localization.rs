//! Localization tests: Smart Strings, CSV, fallback chains and components following the
//! selected locale.

use oxide_localization::*;
use oxide_scene::World;

#[test]
fn smart_strings_format_like_smartformat() {
    let args = [Arg::named("player", "Ace"), Arg::new(3)];
    assert_eq!(
        format_str("Hi {player}, lap {1}", &args, false),
        "Hi Ace, lap 3"
    );
    // Plain strings ignore formatters; smart ones apply them.
    let one = [Arg::new(1)];
    let many = [Arg::new(5)];
    let t = "{0} {0:plural:apple|apples}";
    assert_eq!(format_str(t, &one, true), "1 apple");
    assert_eq!(format_str(t, &many, true), "5 apples");
    assert_eq!(
        format_str("{0:plural:none|one {}|{} items}", &[Arg::new(0)], true),
        "none"
    );
    assert_eq!(
        format_str("{0:plural:none|one {}|{} items}", &many, true),
        "5 items"
    );
    assert_eq!(
        format_str("{0:choose(1|2):gold|silver|other}", &[Arg::new(2)], true),
        "silver"
    );
    assert_eq!(
        format_str("{0:choose(1|2):gold|silver|other}", &[Arg::new(9)], true),
        "other"
    );
    assert_eq!(
        format_str("{0:cond:yes|no}", &[Arg::new("true")], true),
        "yes"
    );
    // Escapes and unknown placeholders stay.
    assert_eq!(format_str("{{0}} {missing}", &args, false), "{0} {missing}");
}

#[test]
fn csv_round_trips() {
    let mut t = StringTableCollection::default();
    t.entry_mut("start").set("en", "Start");
    t.entry_mut("start").set("fr", "Démarrer");
    t.entry_mut("quote").set("en", "Say \"hi\", friend");
    let locales = vec!["en".to_string(), "fr".to_string()];
    let csv = t.to_csv(&locales);
    let mut back = StringTableCollection::default();
    assert_eq!(back.import_csv(&csv).unwrap(), 4);
    assert_eq!(
        back.entry("quote").unwrap().get("en").unwrap().value,
        "Say \"hi\", friend"
    );
    assert_eq!(
        back.entry("start").unwrap().get("fr").unwrap().value,
        "Démarrer"
    );
}

fn settings() -> LocalizationSettings {
    LocalizationSettings {
        locales: vec![
            LocaleInfo {
                code: "en".into(),
                name: "English".into(),
                fallback: String::new(),
            },
            LocaleInfo {
                code: "fr".into(),
                name: "French".into(),
                fallback: String::new(),
            },
            LocaleInfo {
                code: "fr-CA".into(),
                name: "French (Canada)".into(),
                fallback: "fr".into(),
            },
        ],
        default_locale: "en".into(),
        use_system_language: false,
    }
}

#[test]
fn fallback_chains_end_at_the_default() {
    assert_eq!(
        Localization::chain(&settings(), "fr-CA"),
        vec!["fr-CA", "fr", "en"]
    );
    assert_eq!(Localization::chain(&settings(), "en"), vec!["en"]);
    assert_eq!(Localization::startup_locale(&settings()), "en");
}

#[test]
fn components_follow_the_selected_locale() {
    let dir = std::env::temp_dir().join(format!("oxide-loc-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("Assets")).unwrap();
    let mut assets = oxide_assets::AssetServer::new(&dir);
    register_assets(&mut assets);
    let mut table = StringTableCollection {
        table_name: "UI".into(),
        entries: Vec::new(),
    };
    table.entry_mut("laps").set("en", "Lap {0} of {1}");
    table.entry_mut("laps").set("fr", "Tour {0} sur {1}");
    table.entry_mut("only_en").set("en", "English only");
    assets
        .save_data("Assets/UI.stringtable", Box::new(table))
        .unwrap();
    let mut w = World::new();
    w.resources.insert(assets);
    w.resources.insert(settings());
    install(&mut w);
    let id = w.create("Lap");
    w.add_component(id, oxide_text::TextMeshPro::default());
    w.add_component(
        id,
        LocalizeStringEvent {
            table: "UI".into(),
            entry: "laps".into(),
            arguments: vec!["2".into(), "3".into()],
            ..Default::default()
        },
    );
    apply(&mut w);
    assert_eq!(
        w.get_component::<oxide_text::TextMeshPro>(id).unwrap().text,
        "Lap 2 of 3"
    );
    w.set_locale("fr-CA");
    apply(&mut w);
    // fr-CA falls back to French.
    assert_eq!(
        w.get_component::<oxide_text::TextMeshPro>(id).unwrap().text,
        "Tour 2 sur 3"
    );
    // ... and to the default for keys French lacks.
    assert_eq!(w.localized("UI", "only_en", &[]), "English only");
    assert_eq!(
        w.localized("UI", "nope", &[]),
        "No translation found for 'nope' in UI"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
