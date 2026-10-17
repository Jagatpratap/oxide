//! Smart Strings (Unity's SmartFormat): `{0}` and `{name}` placeholders, plural and choose
//! formatters, nested `{}` in their branches.
//! Reference: <https://docs.unity3d.com/Packages/com.unity.localization@1.5/manual/Smart/SmartStrings.html>

/// One argument: positional (`{0}`) and optionally named (`{score}`).
#[derive(Clone, Debug, PartialEq)]
pub struct Arg {
    pub name: Option<String>,
    pub value: String,
}

impl Arg {
    pub fn new(value: impl ToString) -> Self {
        Self {
            name: None,
            value: value.to_string(),
        }
    }

    pub fn named(name: &str, value: impl ToString) -> Self {
        Self {
            name: Some(name.into()),
            value: value.to_string(),
        }
    }
}

fn lookup<'a>(args: &'a [Arg], selector: &str) -> Option<&'a str> {
    if let Ok(i) = selector.trim().parse::<usize>() {
        return args.get(i).map(|a| a.value.as_str());
    }
    args.iter()
        .find(|a| a.name.as_deref() == Some(selector.trim()))
        .map(|a| a.value.as_str())
}

/// Splits on `|` outside nested braces.
fn branches(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            '|' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// The plural branch for `n` (English rules: one / other; with three branches zero / one /
/// other, like SmartFormat).
fn plural_index(n: f64, count: usize) -> usize {
    match count {
        0 | 1 => 0,
        2 => {
            if n == 1.0 {
                0
            } else {
                1
            }
        }
        _ => {
            if n == 0.0 {
                0
            } else if n == 1.0 {
                1
            } else {
                2.min(count - 1)
            }
        }
    }
}

/// Formats one placeholder body (`0`, `name`, `0:plural:a|b`, `0:choose(1|2):a|b`).
fn placeholder(body: &str, args: &[Arg], smart: bool) -> Option<String> {
    let (selector, rest) = match body.split_once(':') {
        Some((s, r)) if smart => (s, Some(r)),
        _ => (body, None),
    };
    let value = lookup(args, selector)?;
    let Some(rest) = rest else {
        return Some(value.to_owned());
    };
    let (formatter, options) = match rest.split_once(':') {
        Some((f, o)) => (f, o),
        None => ("", rest),
    };
    let pick = |branch: &str| format_str(&branch.replace("{}", value), args, smart);
    if formatter == "plural" || formatter == "p" {
        let n: f64 = value.trim().parse().unwrap_or(0.0);
        let b = branches(options);
        return Some(pick(b[plural_index(n, b.len())]));
    }
    if let Some(choices) = formatter
        .strip_prefix("choose(")
        .and_then(|c| c.strip_suffix(')'))
    {
        let keys: Vec<&str> = choices.split('|').collect();
        let b = branches(options);
        let k = keys
            .iter()
            .position(|c| c.trim() == value.trim())
            .unwrap_or(b.len().saturating_sub(1));
        return Some(pick(b.get(k).copied().unwrap_or("")));
    }
    if formatter == "cond" || formatter.is_empty() {
        // Conditional: numbers pick by sign (negative|zero|positive or zero|other),
        // booleans by truth.
        let b = branches(options);
        let k = match value.trim() {
            "true" | "True" => 0,
            "false" | "False" => 1.min(b.len() - 1),
            v => match v.parse::<f64>() {
                Ok(n) if b.len() >= 3 => {
                    if n < 0.0 {
                        0
                    } else if n == 0.0 {
                        1
                    } else {
                        2
                    }
                }
                Ok(n) => (n != 0.0) as usize,
                Err(_) => (!v.is_empty()) as usize,
            },
        };
        return Some(pick(b.get(k.min(b.len() - 1)).copied().unwrap_or("")));
    }
    Some(value.to_owned())
}

/// Replaces placeholders in `text`. Plain strings take `{0}`/`{name}`; smart strings also
/// take formatters. Unknown placeholders and `{{`/`}}` escapes are kept as text.
pub fn format_str(text: &str, args: &[Arg], smart: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '{' && chars.get(i + 1) == Some(&'{') {
            out.push('{');
            i += 2;
            continue;
        }
        if c == '}' && chars.get(i + 1) == Some(&'}') {
            out.push('}');
            i += 2;
            continue;
        }
        if c == '{' {
            // The matching close brace, allowing nesting.
            let mut depth = 0;
            let mut j = i;
            while j < chars.len() {
                match chars[j] {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            if j < chars.len() {
                let body: String = chars[i + 1..j].iter().collect();
                match placeholder(&body, args, smart) {
                    Some(v) => out.push_str(&v),
                    None => {
                        out.push('{');
                        out.push_str(&body);
                        out.push('}');
                    }
                }
                i = j + 1;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}
