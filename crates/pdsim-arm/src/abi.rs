//! Decoding calls according to the ARM procedure call standard with VFP
//! arguments (AAPCS-VFP, what `-mfloat-abi=hard` uses).

use crate::layout::{Kind, Signature};

/// A decoded argument.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Arg {
    I(u32),
    F(f32),
    D(f64),
    I64(u64),
    PdRect([f32; 4]),
    LcdRect([i32; 4]),
}

impl Arg {
    pub fn u(self) -> u32 {
        match self {
            Arg::I(v) => v,
            Arg::F(f) => f as i32 as u32,
            Arg::D(d) => d as i32 as u32,
            Arg::I64(v) => v as u32,
            _ => 0,
        }
    }
    pub fn i(self) -> i32 {
        self.u() as i32
    }
    pub fn f(self) -> f32 {
        match self {
            Arg::F(f) => f,
            Arg::D(d) => d as f32,
            Arg::I(v) => v as i32 as f32,
            _ => 0.0,
        }
    }
}

/// Register and stack access the decoder needs.
pub trait Machine {
    fn core(&self, n: usize) -> u32;
    fn sreg(&self, n: usize) -> f32;
    fn sp(&self) -> u32;
    fn read_u32(&self, addr: u32) -> u32;
}

/// Walks argument slots in order.
pub struct Cursor<'a, M: Machine> {
    m: &'a M,
    ncrn: usize,
    nsaa: u32,
    s_used: u32,
    vfp_done: bool,
    variadic: bool,
}

impl<'a, M: Machine> Cursor<'a, M> {
    pub fn new(m: &'a M, variadic: bool) -> Self {
        Cursor {
            m,
            ncrn: 0,
            nsaa: m.sp(),
            s_used: 0,
            vfp_done: false,
            variadic,
        }
    }

    fn word(&mut self) -> u32 {
        if self.ncrn < 4 {
            self.ncrn += 1;
            self.m.core(self.ncrn - 1)
        } else {
            let v = self.m.read_u32(self.nsaa);
            self.nsaa += 4;
            v
        }
    }

    fn dword(&mut self) -> u64 {
        if self.ncrn % 2 == 1 {
            self.ncrn += 1;
        }
        if self.ncrn <= 2 {
            let lo = self.m.core(self.ncrn) as u64;
            let hi = self.m.core(self.ncrn + 1) as u64;
            self.ncrn += 2;
            lo | (hi << 32)
        } else {
            self.ncrn = 4;
            self.nsaa = (self.nsaa + 7) & !7;
            let lo = self.m.read_u32(self.nsaa) as u64;
            let hi = self.m.read_u32(self.nsaa + 4) as u64;
            self.nsaa += 8;
            lo | (hi << 32)
        }
    }

    /// Allocates `n` consecutive single-precision registers aligned to `align`.
    fn sregs(&mut self, n: u32, align: u32) -> Option<usize> {
        if self.vfp_done {
            return None;
        }
        let mask = (1u32 << n) - 1;
        let mut i = 0;
        while i + n <= 16 {
            if self.s_used & (mask << i) == 0 {
                self.s_used |= mask << i;
                return Some(i as usize);
            }
            i += align;
        }
        self.vfp_done = true;
        None
    }

    /// The next argument of kind `k`.
    pub fn next(&mut self, k: Kind) -> Arg {
        match k {
            Kind::Int => Arg::I(self.word()),
            Kind::Int64 => Arg::I64(self.dword()),
            Kind::LcdRect => Arg::LcdRect([
                self.word() as i32,
                self.word() as i32,
                self.word() as i32,
                self.word() as i32,
            ]),
            Kind::Float if self.variadic => Arg::D(f64::from_bits(self.dword())),
            Kind::Double if self.variadic => Arg::D(f64::from_bits(self.dword())),
            Kind::Float => match self.sregs(1, 1) {
                Some(s) => Arg::F(self.m.sreg(s)),
                None => Arg::F(f32::from_bits(self.word_stack())),
            },
            Kind::Double => match self.sregs(2, 2) {
                Some(s) => {
                    let lo = self.m.sreg(s).to_bits() as u64;
                    let hi = self.m.sreg(s + 1).to_bits() as u64;
                    Arg::D(f64::from_bits(lo | hi << 32))
                }
                None => {
                    self.nsaa = (self.nsaa + 7) & !7;
                    let lo = self.word_stack() as u64;
                    let hi = self.word_stack() as u64;
                    Arg::D(f64::from_bits(lo | hi << 32))
                }
            },
            Kind::PdRect => match self.sregs(4, 1) {
                Some(s) => Arg::PdRect([
                    self.m.sreg(s),
                    self.m.sreg(s + 1),
                    self.m.sreg(s + 2),
                    self.m.sreg(s + 3),
                ]),
                None => {
                    let v = [0, 1, 2, 3].map(|_| f32::from_bits(self.word_stack()));
                    Arg::PdRect(v)
                }
            },
        }
    }

    fn word_stack(&mut self) -> u32 {
        let v = self.m.read_u32(self.nsaa);
        self.nsaa += 4;
        v
    }

    /// Next variadic integer (`%d`, `%s`, `%p`, ...).
    pub fn var_int(&mut self) -> u32 {
        self.word()
    }

    /// Next variadic 64-bit integer (`%lld`).
    pub fn var_i64(&mut self) -> u64 {
        self.dword()
    }

    /// Next variadic double (`%f`; floats are promoted).
    pub fn var_double(&mut self) -> f64 {
        f64::from_bits(self.dword())
    }
}

/// Decodes the fixed arguments of `sig`; the cursor is left after them for
/// reading variadic arguments.
pub fn decode<'a, M: Machine>(m: &'a M, sig: &Signature) -> (Vec<Arg>, Cursor<'a, M>) {
    let mut c = Cursor::new(m, sig.variadic);
    let args = sig.params.iter().map(|k| c.next(*k)).collect();
    (args, c)
}

/// C `printf` formatting, reading arguments from the cursor. `read_str`
/// fetches a C string from emulated memory.
pub fn format<M: Machine>(fmt: &str, c: &mut Cursor<'_, M>, read_str: &dyn Fn(u32) -> String) -> String {
    let mut out = String::new();
    let mut it = fmt.chars().peekable();
    while let Some(ch) = it.next() {
        if ch != '%' {
            out.push(ch);
            continue;
        }
        let mut spec = String::new();
        let mut longs = 0;
        let conv = loop {
            match it.next() {
                None => break None,
                Some(x) if "-+ #0".contains(x) || x.is_ascii_digit() || x == '.' => spec.push(x),
                Some('*') => spec.push_str(&(c.var_int() as i32).to_string()),
                Some('l') => longs += 1,
                Some('h') | Some('z') | Some('j') | Some('t') => {}
                Some(x) => break Some(x),
            }
        };
        let Some(conv) = conv else { break };
        let left = spec.contains('-');
        let zero = spec.starts_with('0') || spec.contains("+0") || spec.contains(" 0");
        let digits: String = spec
            .chars()
            .filter(|c| *c != '-' && *c != '+' && *c != ' ' && *c != '#')
            .collect();
        let (w, p) = match digits.split_once('.') {
            Some((w, p)) => (
                w.trim_start_matches('0').parse::<usize>().unwrap_or(0),
                Some(p.parse::<usize>().unwrap_or(0)),
            ),
            None => (digits.trim_start_matches('0').parse::<usize>().unwrap_or(0), None),
        };
        let body = match conv {
            '%' => "%".to_string(),
            'd' | 'i' => {
                if longs >= 2 {
                    (c.var_i64() as i64).to_string()
                } else {
                    (c.var_int() as i32).to_string()
                }
            }
            'u' => {
                if longs >= 2 {
                    c.var_i64().to_string()
                } else {
                    c.var_int().to_string()
                }
            }
            'x' => format!(
                "{:x}",
                if longs >= 2 {
                    c.var_i64()
                } else {
                    c.var_int() as u64
                }
            ),
            'X' => format!(
                "{:X}",
                if longs >= 2 {
                    c.var_i64()
                } else {
                    c.var_int() as u64
                }
            ),
            'o' => format!("{:o}", c.var_int()),
            'p' => format!("0x{:x}", c.var_int()),
            'c' => char::from_u32(c.var_int()).map(String::from).unwrap_or_default(),
            's' => {
                let s = read_str(c.var_int());
                match p {
                    Some(p) => s.chars().take(p).collect(),
                    None => s,
                }
            }
            'f' | 'F' => format!("{:.*}", p.unwrap_or(6), c.var_double()),
            'e' | 'E' => {
                let s = format!("{:.*e}", p.unwrap_or(6), c.var_double());
                if conv == 'E' {
                    s.to_uppercase()
                } else {
                    s
                }
            }
            'g' | 'G' => {
                let v = c.var_double();
                let s = format!("{v}");
                if s.len() > 12 {
                    format!("{v:e}")
                } else {
                    s
                }
            }
            other => format!("%{other}"),
        };
        if body.len() < w {
            let pad = w - body.len();
            if left {
                out.push_str(&body);
                out.extend(std::iter::repeat_n(' ', pad));
            } else if zero && "dixXuf".contains(conv) {
                let (sign, digits) = if let Some(d) = body.strip_prefix('-') {
                    ("-", d)
                } else {
                    ("", body.as_str())
                };
                out.push_str(sign);
                out.extend(std::iter::repeat_n('0', pad));
                out.push_str(digits);
            } else {
                out.extend(std::iter::repeat_n(' ', pad));
                out.push_str(&body);
            }
        } else {
            out.push_str(&body);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Kind::*;

    struct M {
        r: [u32; 4],
        s: [f32; 16],
        stack: Vec<u32>,
    }

    impl Machine for M {
        fn core(&self, n: usize) -> u32 {
            self.r[n]
        }
        fn sreg(&self, n: usize) -> f32 {
            self.s[n]
        }
        fn sp(&self) -> u32 {
            0x1000
        }
        fn read_u32(&self, addr: u32) -> u32 {
            self.stack[((addr - 0x1000) / 4) as usize]
        }
    }

    #[test]
    fn draw_ellipse_signature() {
        // drawEllipse(int x, y, w, h, lineWidth, float start, float end, LCDColor color)
        let m = M {
            r: [1, 2, 3, 4],
            s: [
                10.0, 20.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
            stack: vec![5, 1],
        };
        let sig = Signature {
            ret: None,
            params: vec![Int, Int, Int, Int, Int, Float, Float, Int],
            variadic: false,
        };
        let (a, _) = decode(&m, &sig);
        assert_eq!(
            a,
            vec![
                Arg::I(1),
                Arg::I(2),
                Arg::I(3),
                Arg::I(4),
                Arg::I(5),
                Arg::F(10.0),
                Arg::F(20.0),
                Arg::I(1)
            ]
        );
    }

    #[test]
    fn pdrect_hfa_and_backfill() {
        // f(float a, PDRect r, float b): a=s0, r=s1..s4, b=s5
        let mut s = [0.0; 16];
        for (i, v) in s.iter_mut().enumerate() {
            *v = i as f32;
        }
        let m = M {
            r: [0; 4],
            s,
            stack: vec![],
        };
        let sig = Signature {
            ret: None,
            params: vec![Float, PdRect, Float],
            variadic: false,
        };
        let (a, _) = decode(&m, &sig);
        assert_eq!(
            a,
            vec![Arg::F(0.0), Arg::PdRect([1.0, 2.0, 3.0, 4.0]), Arg::F(5.0)]
        );
        // f(double d, float f): d = s0/s1, f backfills? (s2: no back-fill gap here)
        let sig = Signature {
            ret: None,
            params: vec![Float, Double, Float],
            variadic: false,
        };
        let (a, _) = decode(&m, &sig);
        // float -> s0, double -> s2/s3 (aligned), float back-fills s1
        assert_eq!(a[2], Arg::F(1.0));
    }

    #[test]
    fn printf_formats() {
        let d = 2.5f64.to_bits();
        let m = M {
            r: [0, 42, 0xff, 7],
            s: [0.0; 16],
            stack: vec![d as u32, (d >> 32) as u32, 0x2000],
        };
        let sig = Signature {
            ret: None,
            params: vec![Int],
            variadic: true,
        };
        let (_, mut c) = decode(&m, &sig);
        let s = format("n=%d x=%04x c=%c f=%.2f s=%s %%", &mut c, &|a| {
            if a == 0x2000 {
                "hi".into()
            } else {
                "?".into()
            }
        });
        assert_eq!(s, "n=42 x=00ff c=\u{7} f=2.50 s=hi %");
    }
}
