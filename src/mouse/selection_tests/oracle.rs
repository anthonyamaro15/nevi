use super::*;
use serde_json::{Value, json};

fn snapshot(editor: &Editor) -> Value {
    json!({
        "text": editor.buffer().content(),
        "mode": match editor.mode { Mode::Normal => "n", Mode::Insert => "i", Mode::Replace => "R", Mode::Visual => "v", _ => "other" },
        "cursor": [editor.cursor.line, editor.cursor.col],
        "anchor": if editor.mode.is_visual() { json!([editor.visual.anchor_line, editor.visual.anchor_col]) } else { Value::Null },
    })
}

#[test]
#[ignore = "Requires Neovim; run with NEVI_VIM_ORACLE=1"]
fn vim_oracle_smoke_mouse_selection_matches_neovim() {
    if std::env::var("NEVI_VIM_ORACLE").as_deref() != Ok("1") {
        return;
    }
    let mut cases = Vec::new();
    for entry in ["", "i", "R", "i<C-o>"] {
        for op in ["y", "d", "c", "<Esc>"] {
            let mut actions = Vec::new();
            if !entry.is_empty() {
                actions.push(json!(["key", entry]));
            }
            actions.extend([
                json!(["mouse", "press", 1, 0]),
                json!(["mouse", "drag", 8, 0]),
                json!(["mouse", "release", 4, 0]),
                json!(["key", op]),
            ]);
            cases.push(json!({"name": format!("entry={entry}, operator={op}"), "text": "alpha beta gamma\nsecond line\n", "actions": actions}));
        }
    }
    for (name, text, actions) in [
        (
            "stationary",
            "alpha\n",
            json!([
                ["mouse", "press", 1, 0],
                ["mouse", "drag", 1, 0],
                ["mouse", "release", 1, 0]
            ]),
        ),
        (
            "release without drag",
            "alpha\n",
            json!([
                ["mouse", "press", 1, 0],
                ["mouse", "release", 4, 0],
                ["key", "y"]
            ]),
        ),
        (
            "reverse multiline",
            "alpha\nsecond line\n",
            json!([
                ["mouse", "press", 3, 1],
                ["mouse", "drag", 1, 0],
                ["mouse", "release", 1, 0],
                ["key", "d"]
            ]),
        ),
        (
            "wide characters",
            "日本語abc\n",
            json!([
                ["mouse", "press", 2, 0],
                ["mouse", "drag", 6, 0],
                ["mouse", "release", 6, 0],
                ["key", "y"]
            ]),
        ),
        (
            "line break",
            "alpha\nbeta\n",
            json!([
                ["mouse", "press", 1, 0],
                ["mouse", "drag", 70, 0],
                ["mouse", "release", 70, 0],
                ["key", "d"],
                ["key", "u"]
            ]),
        ),
        (
            "end of file",
            "alpha\nbeta\n",
            json!([
                ["mouse", "press", 1, 0],
                ["mouse", "drag", 70, 1],
                ["mouse", "release", 70, 1],
                ["key", "d"],
                ["key", "u"]
            ]),
        ),
    ] {
        cases.push(json!({"name": name, "text": text, "actions": actions}));
    }
    cases.push(json!({"name": "wrapped rows", "text": "abcdefghijklmnopqrst\nnext\n", "width": 12, "wrap": true,
        "actions": [["mouse", "press", 1, 0], ["mouse", "drag", 2, 1], ["mouse", "release", 2, 1], ["key", "d"]]}));

    for (name, entry, end_row) in [
        ("counted Insert", "3iX", 0),
        ("block Insert", "<C-v>jIX", 1),
        ("counted Replace", "3RX", 0),
    ] {
        cases.push(json!({"name": name, "text": "alpha beta gamma\nsecond line\n",
            "actions": [["key", entry], ["mouse", "press", 2, 0], ["mouse", "drag", 5, end_row], ["mouse", "release", 5, end_row], ["key", "d"]]}));
    }
    cases.push(json!({"name": "Escape past EOL", "text": "alpha\nbeta\n",
        "actions": [["mouse", "press", 1, 0], ["mouse", "drag", 70, 0], ["mouse", "release", 70, 0], ["key", "<Esc>x"]]}));
    cases.push(json!({"name": "stationary past EOL", "text": "alpha\n",
        "actions": [["mouse", "press", 70, 0], ["mouse", "drag", 70, 0], ["mouse", "release", 70, 0]]}));

    let script =
        include_str!("oracle.lua").replace("__CASES__", &serde_json::to_string(&cases).unwrap());
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "nevi-mouse-oracle-{}-{unique}.lua",
        std::process::id()
    ));
    std::fs::write(&path, script).unwrap();
    let output = std::process::Command::new("nvim")
        .args(["--clean", "--headless", "-n", "-i", "NONE", "-l"])
        .arg(&path)
        .output();
    let _ = std::fs::remove_file(path);
    let output = output.expect("start Neovim mouse oracle");
    assert!(
        output.status.success(),
        "Neovim oracle: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected: Vec<Vec<Value>> =
        serde_json::from_slice(&output.stdout).expect("Neovim snapshots");
    assert_eq!(expected.len(), cases.len());
    for (case, snapshots) in cases.iter().zip(expected) {
        let mut editor = editor(case["text"].as_str().unwrap());
        editor.set_size(case["width"].as_u64().unwrap_or(80) as u16, 24);
        editor.settings.editor.wrap = case["wrap"].as_bool().unwrap_or(false);
        editor.settings.editor.wrap_width = 9999;
        let actions = case["actions"].as_array().unwrap();
        assert_eq!(snapshots.len(), actions.len());
        for (action, expected) in actions.iter().zip(snapshots) {
            if action[0] == "key" {
                for key in
                    crate::input::key_notation::parse_key_sequence(action[1].as_str().unwrap())
                        .unwrap()
                {
                    handle_key(&mut editor, key);
                }
            } else {
                let kind = match action[1].as_str().unwrap() {
                    "press" => MouseEventKind::Down(MouseButton::Left),
                    "drag" => MouseEventKind::Drag(MouseButton::Left),
                    "release" => MouseEventKind::Up(MouseButton::Left),
                    _ => unreachable!(),
                };
                mouse(
                    &mut editor,
                    kind,
                    action[2].as_u64().unwrap() as u16,
                    action[3].as_u64().unwrap() as u16,
                );
            }
            assert_eq!(
                snapshot(&editor),
                expected,
                "{} after {action}",
                case["name"]
            );
        }
    }
}
