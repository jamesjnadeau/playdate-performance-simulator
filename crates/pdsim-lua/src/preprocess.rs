//! The Playdate Lua dialect: compound assignment operators.
//!
//! `pdc` accepts `+= -= *= /= //= %= <<= >>= &= |= ^=`. This rewrites
//! `lhs op= rhs` into `lhs = lhs op (rhs)`, keeping every newline so line
//! numbers in errors and profiles still match the source.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Name,
    Number,
    Str,
    Op,
    Compound,
    Open,
    Close,
    Comment,
    Space,
    Newline,
}

#[derive(Debug, Clone, Copy)]
struct Tok {
    kind: Kind,
    start: usize,
    end: usize,
}

const COMPOUND: [&str; 11] = [
    "//=", "<<=", ">>=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=",
];
const OPS: [&str; 16] = [
    "...", "..", "==", "~=", "<=", ">=", "<<", ">>", "//", "::", "+", "-", "*", "/", "%", "^",
];

fn long_bracket_level(s: &[u8], i: usize) -> Option<usize> {
    if s.get(i) != Some(&b'[') {
        return None;
    }
    let mut j = i + 1;
    while s.get(j) == Some(&b'=') {
        j += 1;
    }
    (s.get(j) == Some(&b'[')).then_some(j - i - 1)
}

fn skip_long(s: &[u8], i: usize, level: usize) -> usize {
    let close: Vec<u8> = std::iter::once(b']')
        .chain(std::iter::repeat_n(b'=', level))
        .chain(std::iter::once(b']'))
        .collect();
    let body = i + level + 2;
    s[body.min(s.len())..]
        .windows(close.len())
        .position(|w| w == close.as_slice())
        .map_or(s.len(), |p| body + p + close.len())
}

fn tokenize(src: &str) -> Vec<Tok> {
    let s = src.as_bytes();
    let mut toks = vec![];
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        let start = i;
        let kind = if c == b'\n' {
            i += 1;
            Kind::Newline
        } else if c.is_ascii_whitespace() {
            while i < s.len() && s[i].is_ascii_whitespace() && s[i] != b'\n' {
                i += 1;
            }
            Kind::Space
        } else if s[i..].starts_with(b"--") {
            if let Some(level) = long_bracket_level(s, i + 2) {
                i = skip_long(s, i + 2, level);
            } else {
                while i < s.len() && s[i] != b'\n' {
                    i += 1;
                }
            }
            Kind::Comment
        } else if let Some(level) = long_bracket_level(s, i) {
            i = skip_long(s, i, level);
            Kind::Str
        } else if c == b'"' || c == b'\'' {
            i += 1;
            while i < s.len() && s[i] != c && s[i] != b'\n' {
                if s[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(s.len());
            Kind::Str
        } else if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 {
            while i < s.len() && (s[i].is_ascii_alphanumeric() || s[i] == b'_' || s[i] >= 0x80) {
                i += 1;
            }
            Kind::Name
        } else if c.is_ascii_digit() || (c == b'.' && s.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            while i < s.len()
                && (s[i].is_ascii_alphanumeric()
                    || s[i] == b'.'
                    || ((s[i] == b'-' || s[i] == b'+') && matches!(s[i - 1], b'e' | b'E' | b'p' | b'P')))
            {
                i += 1;
            }
            Kind::Number
        } else if matches!(c, b'(' | b'[' | b'{') {
            i += 1;
            Kind::Open
        } else if matches!(c, b')' | b']' | b'}') {
            i += 1;
            Kind::Close
        } else if let Some(op) = COMPOUND.iter().find(|op| s[i..].starts_with(op.as_bytes())) {
            i += op.len();
            Kind::Compound
        } else if let Some(op) = OPS.iter().find(|op| s[i..].starts_with(op.as_bytes())) {
            i += op.len();
            Kind::Op
        } else {
            i += 1;
            Kind::Op
        };
        toks.push(Tok { kind, start, end: i });
    }
    toks
}

const STMT_KEYWORDS: [&str; 13] = [
    "end", "else", "elseif", "until", "local", "return", "if", "for", "while", "repeat", "do", "then", "goto",
];
const EXPR_KEYWORDS: [&str; 6] = ["and", "or", "not", "nil", "true", "false"];

fn text<'a>(src: &'a str, t: &Tok) -> &'a str {
    &src[t.start..t.end]
}

fn is_binary_op(src: &str, t: &Tok) -> bool {
    match t.kind {
        Kind::Op => !matches!(text(src, t), ";" | "," | "=" | ":" | "::" | "#"),
        Kind::Name => matches!(text(src, t), "and" | "or"),
        _ => false,
    }
}

fn ends_value(src: &str, t: &Tok) -> bool {
    match t.kind {
        Kind::Number | Kind::Str | Kind::Close => true,
        Kind::Name => !STMT_KEYWORDS.contains(&text(src, t)) && !matches!(text(src, t), "and" | "or" | "not"),
        _ => false,
    }
}

fn starts_value(src: &str, t: &Tok) -> bool {
    match t.kind {
        Kind::Number | Kind::Str => true,
        Kind::Name => !matches!(text(src, t), "and" | "or"),
        _ => false,
    }
}

/// Rewrites compound assignments. Returns the source unchanged when it has
/// none.
pub fn preprocess(src: &str) -> String {
    let toks = tokenize(src);
    if !toks.iter().any(|t| t.kind == Kind::Compound) {
        return src.to_string();
    }
    // (byte offset, text) insertions and (start, end, text) replacements.
    let mut edits: Vec<(usize, usize, String)> = vec![];
    let significant = |k: Kind| !matches!(k, Kind::Space | Kind::Comment | Kind::Newline);
    for (ci, t) in toks.iter().enumerate() {
        if t.kind != Kind::Compound {
            continue;
        }
        // Left-hand side: walk back over a suffixed expression
        // (`name`, `.name`, `:name`, `[...]`, `(...)`).
        #[derive(PartialEq)]
        enum S {
            Start,
            AfterName,
            AfterDot,
            AfterGroup,
        }
        let mut state = S::Start;
        let mut lhs_start = None;
        let mut j = ci;
        while j > 0 {
            j -= 1;
            let p = toks[j];
            if !significant(p.kind) {
                if p.kind == Kind::Newline && matches!(state, S::AfterName | S::AfterGroup) {
                    break;
                }
                continue;
            }
            let tx = text(src, &p);
            let plain_name = p.kind == Kind::Name
                && !STMT_KEYWORDS.contains(&tx)
                && !EXPR_KEYWORDS.contains(&tx)
                && tx != "function";
            match state {
                S::Start | S::AfterDot | S::AfterGroup if plain_name => {
                    lhs_start = Some(p.start);
                    state = S::AfterName;
                }
                S::Start | S::AfterDot | S::AfterGroup if p.kind == Kind::Close => {
                    // Skip back to the matching bracket.
                    let mut depth = 1;
                    while j > 0 && depth > 0 {
                        j -= 1;
                        match toks[j].kind {
                            Kind::Close => depth += 1,
                            Kind::Open => depth -= 1,
                            _ => {}
                        }
                    }
                    lhs_start = Some(toks[j].start);
                    state = S::AfterGroup;
                }
                S::AfterName if p.kind == Kind::Op && matches!(tx, "." | ":") => state = S::AfterDot,
                _ => break,
            }
        }
        if state == S::AfterDot {
            continue;
        }
        let Some(lhs_start) = lhs_start else { continue };
        let lhs = src[lhs_start..t.start].trim_end().to_string();
        let op = &text(src, t)[..t.end - t.start - 1];
        // Right-hand side: until the expression ends.
        let mut k = ci + 1;
        let mut depth = 0i32;
        let mut last: Option<Tok> = None;
        let mut end = src.len();
        while k < toks.len() {
            let q = toks[k];
            if !significant(q.kind) {
                if q.kind == Kind::Newline && depth == 0 {
                    // Continue onto the next line only if the expression is
                    // obviously unfinished.
                    let next = toks[k + 1..].iter().find(|x| significant(x.kind));
                    let unfinished = last.is_none_or(|l| is_binary_op(src, &l) || l.kind == Kind::Open)
                        || next.is_some_and(|n| is_binary_op(src, n) && text(src, n) != "-");
                    if !unfinished {
                        end = last.map_or(q.start, |l| l.end);
                        break;
                    }
                }
                k += 1;
                continue;
            }
            if depth == 0 {
                let tx = text(src, &q);
                let stop = matches!(q.kind, Kind::Compound)
                    || (q.kind == Kind::Op && matches!(tx, ";" | "=" | ","))
                    || (q.kind == Kind::Name && STMT_KEYWORDS.contains(&tx) && tx != "function")
                    || q.kind == Kind::Close
                    || last.is_some_and(|l| ends_value(src, &l) && starts_value(src, &q));
                if stop {
                    end = last.map_or(q.start, |l| l.end);
                    break;
                }
            }
            match q.kind {
                Kind::Open => depth += 1,
                Kind::Close => depth -= 1,
                _ => {}
            }
            last = Some(q);
            k += 1;
        }
        if k >= toks.len() {
            end = last.map_or(src.len(), |l| l.end);
        }
        let op_end = match toks.get(ci + 1) {
            Some(n) if n.kind == Kind::Space => n.end,
            _ => t.end,
        };
        edits.push((t.start, op_end, format!("= {lhs} {op} (")));
        edits.push((end, end, ")".to_string()));
    }
    edits.sort_by_key(|e| (e.0, e.1));
    let mut out = String::with_capacity(src.len() + edits.len() * 8);
    let mut pos = 0;
    for (s, e, txt) in edits {
        out.push_str(&src[pos..s]);
        out.push_str(&txt);
        pos = e;
    }
    out.push_str(&src[pos..]);
    out
}

#[cfg(test)]
mod tests {
    use super::preprocess as p;

    #[test]
    fn simple_ops() {
        assert_eq!(p("x += 1"), "x = x + (1)");
        assert_eq!(p("x -= y * 2"), "x = x - (y * 2)");
        assert_eq!(p("x //= 2"), "x = x // (2)");
        assert_eq!(p("x <<= 2"), "x = x << (2)");
        assert_eq!(p("x >>= 1"), "x = x >> (1)");
        assert_eq!(p("x &= 3"), "x = x & (3)");
        assert_eq!(p("x |= 3"), "x = x | (3)");
        assert_eq!(p("x ^= 3"), "x = x ^ (3)");
        assert_eq!(p("x %= 3"), "x = x % (3)");
        assert_eq!(p("x /= 3"), "x = x / (3)");
        assert_eq!(p("x *= 3"), "x = x * (3)");
    }

    #[test]
    fn suffixed_lhs() {
        assert_eq!(p("self.x += dx"), "self.x = self.x + (dx)");
        assert_eq!(p("t[i + 1] -= 2"), "t[i + 1] = t[i + 1] - (2)");
        assert_eq!(p("a.b[c].d *= f(1, 2)"), "a.b[c].d = a.b[c].d * (f(1, 2))");
    }

    #[test]
    fn statement_boundaries() {
        assert_eq!(p("x += 1 y = 2"), "x = x + (1) y = 2");
        assert_eq!(p("x += 1; y += 2"), "x = x + (1); y = y + (2)");
        assert_eq!(p("if a then x += 1 end"), "if a then x = x + (1) end");
        assert_eq!(p("x += 1\ny += 2\n"), "x = x + (1)\ny = y + (2)\n");
        assert_eq!(
            p("local function f() n += 1 end"),
            "local function f() n = n + (1) end"
        );
    }

    #[test]
    fn multiline_rhs_keeps_lines() {
        let src = "x += a +\n  b\nfoo()";
        let out = p(src);
        assert_eq!(out, "x = x + (a +\n  b)\nfoo()");
        assert_eq!(out.lines().count(), src.lines().count());
    }

    #[test]
    fn leaves_strings_comments_and_comparisons_alone() {
        let s = "local s = \"a += b\" -- x += 1\nif a <= b and c >= d and e ~= f and g == h then end";
        assert_eq!(p(s), s);
        let s = "--[[ x += 1 ]] y = [[ z += 1 ]]";
        assert_eq!(p(s), s);
    }

    #[test]
    fn runs_in_lua() {
        let lua = mlua::Lua::new();
        let src = "local t = {x = 1, v = {2}}\nt.x += 4\nt.v[1] *= 3\nlocal n = 10\nn //= 3\nn <<= 2\nreturn t.x, t.v[1], n";
        let (a, b, c): (i64, i64, i64) = lua.load(p(src)).eval().unwrap();
        assert_eq!((a, b, c), (5, 6, 12));
    }
}
