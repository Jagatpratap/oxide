//! UI Toolkit tests: UXML and USS parsing, selectors and the cascade, flexbox layout and
//! the built-in controls driven by pointer and keyboard input.

use egui::{pos2, vec2};
use oxide_uitk::style::{Len, Rgba};
use oxide_uitk::*;
use std::sync::Arc;

const DOC: &str = r#"<ui:UXML xmlns:ui="UnityEngine.UIElements">
  <Style src="menu.uss" />
  <ui:VisualElement name="bar" class="row" style="height: 40px;">
    <ui:Button name="play" text="Play" class="big" />
    <ui:Button name="quit" text="Quit" />
  </ui:VisualElement>
  <ui:Toggle name="sound" label="Sound" value="true" />
  <ui:Slider name="volume" label="Volume" low-value="0" high-value="10" value="5" />
  <ui:TextField name="player" label="Name" value="Ace" />
  <ui:DropdownField name="quality" label="Quality" choices="Low,Medium,High" index="1" />
  <ui:Foldout name="more" text="More">
    <ui:Label name="hint" text="Hello" />
  </ui:Foldout>
</ui:UXML>"#;

const USS: &str = r#"
/* layout */
.row { flex-direction: row; }
Button { flex-grow: 1; color: #ff0000; }
.big { --accent: rgb(0, 255, 0); color: var(--accent); width: 25%; flex-grow: 0; }
#bar > Button:hover { background-color: #0000ff; }
#more Label { font-size: 20px; }
"#;

fn panel() -> (Panel, Vec<SheetRef>) {
    let tree = uxml::parse(DOC, "Assets/UI").expect("uxml");
    assert_eq!(tree.styles, vec!["Assets/UI/menu.uss".to_string()]);
    let (sheet, errors) = uss::parse(USS);
    assert!(errors.is_empty(), "{errors:?}");
    let theme = uss::parse(uss::DEFAULT_THEME).0;
    let sheets = vec![
        SheetRef {
            sheet: Arc::new(theme),
            base: String::new(),
        },
        SheetRef {
            sheet: Arc::new(sheet),
            base: "Assets/UI".into(),
        },
    ];
    (Panel::new(&tree), sheets)
}

/// Runs `f` inside an egui frame (text layout needs fonts).
fn with_ctx(f: impl FnOnce(&egui::Context)) {
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput::default());
    f(&ctx);
    ctx.end_pass().textures_delta.clear();
}

#[test]
fn uxml_round_trips() {
    let tree = uxml::parse(DOC, "Assets/UI").expect("uxml");
    let text = uxml::write(&tree, "Assets/UI");
    let back = uxml::parse(&text, "Assets/UI").expect("written uxml");
    assert_eq!(back, tree);
    let bar = &tree.root.children[0];
    assert_eq!(
        (bar.name.as_str(), bar.classes.clone()),
        ("bar", vec!["row".to_string()])
    );
    assert_eq!(bar.style_value("height"), Some("40px"));
    assert_eq!(bar.children[0].attr("text"), Some("Play"));
}

#[test]
fn selectors_match_and_rank_like_css() {
    let sel = uss::parse_selector("#bar > Button.big:hover").expect("selector");
    assert_eq!(sel.specificity(), (1, 2, 1));
    let (mut p, sheets) = panel();
    let play = p.find("play").expect("play");
    assert_eq!(p.query(".big"), vec![play]);
    assert_eq!(p.query("Button").len(), 2);
    assert_eq!(p.query("#more Label").len(), 1);
    p.style(&sheets);
    // `.big` (class) beats `Button` (type); the variable resolves.
    assert_eq!(p.elements[play].style.color, Rgba([0.0, 1.0, 0.0, 1.0]));
    let quit = p.find("quit").unwrap();
    assert_eq!(p.elements[quit].style.color, Rgba([1.0, 0.0, 0.0, 1.0]));
    assert_eq!(p.elements[play].style.width, Len::Percent(25.0));
    // Inherited font size from the descendant rule.
    let hint = p.find("hint").unwrap();
    assert_eq!(p.elements[hint].style.font_size, 20.0);
    // Theme: buttons get Unity's button background.
    assert!(p.elements[quit].style.background_color.0[3] > 0.0);
}

#[test]
fn flexbox_lays_out_rows_and_percentages() {
    with_ctx(|ctx| {
        let (mut p, sheets) = panel();
        p.style(&sheets);
        p.layout(vec2(400.0, 300.0), ctx);
        let bar = p.elements[p.find("bar").unwrap()].rect;
        let play = p.elements[p.find("play").unwrap()].rect;
        let quit = p.elements[p.find("quit").unwrap()].rect;
        assert_eq!(bar.width(), 400.0);
        assert_eq!(bar.height(), 40.0);
        // 25% of the row (minus margins), then Quit grows into the rest, side by side.
        assert!((play.width() - 100.0).abs() < 1.0, "play {play:?}");
        assert!(quit.left() > play.right() - 0.5, "{play:?} {quit:?}");
        assert!(
            quit.right() <= 400.0 + 0.5 && quit.width() > 250.0,
            "quit {quit:?}"
        );
        // The column: the toggle comes below the bar.
        let sound = p.elements[p.find("sound").unwrap()].rect;
        assert!(sound.top() >= bar.bottom(), "{sound:?}");
        assert!(sound.height() >= 16.0);
    });
}

#[test]
fn controls_respond_to_input() {
    with_ctx(|ctx| {
        let (mut p, sheets) = panel();
        let frame = |p: &mut Panel, input: PanelInput| {
            p.input(&input);
            p.style(&sheets);
            p.layout(vec2(400.0, 300.0), ctx);
        };
        frame(&mut p, PanelInput::default());
        let center = |p: &Panel, n: &str| p.elements[p.find(n).unwrap()].rect.center();
        let click = |p: &mut Panel, at: egui::Pos2| {
            frame(
                p,
                PanelInput {
                    pointer: Some(at),
                    pressed: true,
                    down: true,
                    ..Default::default()
                },
            );
            let ev = p.events.clone();
            frame(
                p,
                PanelInput {
                    pointer: Some(at),
                    released: true,
                    ..Default::default()
                },
            );
            let mut all = ev;
            all.extend(p.events.clone());
            all
        };
        // Button click.
        let at = center(&p, "play");
        let ev = click(&mut p, at);
        assert!(
            ev.iter()
                .any(|e| e.kind == UiEventKind::Click && e.element == "play"),
            "{ev:?}"
        );
        // Hover state styles the button (#bar > Button:hover).
        let at = center(&p, "quit");
        frame(
            &mut p,
            PanelInput {
                pointer: Some(at),
                ..Default::default()
            },
        );
        let quit = p.find("quit").unwrap();
        assert!(p.elements[quit].hover);
        assert_eq!(
            p.elements[quit].style.background_color,
            Rgba([0.0, 0.0, 1.0, 1.0])
        );
        // Toggle flips.
        let at = center(&p, "sound");
        let ev = click(&mut p, at);
        assert!(
            ev.iter()
                .any(|e| e.element == "sound" && e.value == Value::Bool(false)),
            "{ev:?}"
        );
        // Slider: pressing at the right end of the track sets the high value.
        let v = p.elements[p.find("volume").unwrap()].rect;
        frame(
            &mut p,
            PanelInput {
                pointer: Some(pos2(v.right() - 2.0, v.center().y)),
                pressed: true,
                down: true,
                ..Default::default()
            },
        );
        let val = p.elements[p.find("volume").unwrap()]
            .value
            .as_f32()
            .unwrap();
        assert!(val > 9.5, "volume {val}");
        frame(
            &mut p,
            PanelInput {
                released: true,
                ..Default::default()
            },
        );
        // Typing into the text field.
        let at = center(&p, "player");
        click(&mut p, at);
        frame(
            &mut p,
            PanelInput {
                keyboard: vec![
                    KeyInput::Key(egui::Key::Backspace),
                    KeyInput::Text("e!".into()),
                    KeyInput::Key(egui::Key::Backspace),
                    KeyInput::Key(egui::Key::Enter),
                ],
                ..Default::default()
            },
        );
        let f = p.find("player").unwrap();
        assert_eq!(p.elements[f].value, Value::Text("Ace".into()));
        assert!(p.events.iter().any(|e| e.kind == UiEventKind::Submit));
        // Dropdown: open, then pick the third choice below it.
        let d = p.elements[p.find("quality").unwrap()].rect;
        click(&mut p, d.center());
        let ev = click(&mut p, pos2(d.center().x, d.bottom() + d.height() * 2.5));
        assert!(
            ev.iter()
                .any(|e| e.element == "quality" && e.value == Value::Int(2)),
            "{ev:?}"
        );
        // Foldout closes from its header; its label is no longer laid out.
        let fo = p.elements[p.find("more").unwrap()].rect;
        click(&mut p, pos2(fo.left() + 30.0, fo.top() + 6.0));
        let hint = p.find("hint").unwrap();
        assert!(
            !p.elements[hint].rect.is_positive(),
            "{:?}",
            p.elements[hint].rect
        );
    });
}

#[test]
fn panel_settings_scale_with_screen_size() {
    let ps = PanelSettings::default();
    // 1200 x 800 reference, matching width: twice as wide doubles the scale.
    assert!((ps.scale_for(oxide_math::Vector2::new(2400.0, 800.0)) - 2.0).abs() < 1e-4);
    let fixed = PanelSettings {
        scale_mode: PanelScaleMode::ConstantPixelSize,
        scale: 1.5,
        ..Default::default()
    };
    assert_eq!(fixed.scale_for(oxide_math::Vector2::new(100.0, 100.0)), 1.5);
}
