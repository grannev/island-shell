//! Spotlight's answers in the launcher: arithmetic (`2*(17+5)`,
//! `2^10`, `sqrt 2`, `15% от 2400`, `2400 + 15%`) and conversions between
//! units (`5 миль в км`, `100 f to c`, `1.5 GB в MiB`) and currencies
//! (`100 usd в rub`, `50€ в $`). The answer is the launcher's first row;
//! Enter copies the number.
//!
//! Currency rates come from open.er-api.com (no key), fetched by `curl` on
//! a background thread at most twice a day and cached under
//! `$XDG_CACHE_HOME/island-shell/rates.json`; without them only currencies
//! are missing.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// An answer for the launcher.
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    /// what the row shows ("8,05 км", "1 024")
    pub shown: String,
    /// what Enter copies ("8.05", "1024")
    pub copy: String,
    /// the question, tidied ("5 миль → км")
    pub detail: String,
}

/// The answer to `query`, if it is a calculation or a conversion (a bare
/// number or a word is not).
pub fn answer(query: &str) -> Option<Answer> {
    let query = query.trim();
    if query.is_empty() || query.chars().count() > 120 {
        return None;
    }
    let rates = rates();
    if let Some(answer) = convert(query, rates.as_deref()) {
        return Some(answer);
    }
    let value = percent_of(query).or_else(|| {
        let tokens = tokenize(query)?;
        // a lone number or constant asks nothing
        if !tokens.iter().any(|t| matches!(t, Token::Op(_) | Token::Open | Token::Ident(_)))
            || matches!(tokens.as_slice(), [Token::Ident(_)])
        {
            return None;
        }
        let mut parser = Parser { tokens, at: 0 };
        let value = parser.expr()?;
        (parser.at == parser.tokens.len()).then_some(value.0)
    })?;
    value.is_finite().then(|| Answer { shown: shown(value), copy: plain(value), detail: format!("{query} =") })
}

// ---- arithmetic -----------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Num(f64),
    Op(char),
    Open,
    Close,
    Ident(String),
}

fn tokenize(s: &str) -> Option<Vec<Token>> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || (c == '.' || c == ',') && chars.get(i + 1).is_some_and(char::is_ascii_digit) {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == ',' || chars[i] == '_') {
                i += 1;
            }
            // scientific notation: 1e6, 2.5E-3
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E')
                && chars.get(i + 1).is_some_and(|c| c.is_ascii_digit() || *c == '-' || *c == '+')
            {
                i += 2;
                while i < chars.len() && chars[i].is_ascii_digit() { i += 1; }
            }
            let text: String = chars[start..i].iter().filter(|c| **c != '_').collect();
            out.push(Token::Num(text.replace(',', ".").parse().ok()?));
        } else if c.is_alphabetic() || c == 'π' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == 'π') { i += 1; }
            out.push(Token::Ident(chars[start..i].iter().collect::<String>().to_lowercase()));
        } else {
            out.push(match c {
                '(' => Token::Open,
                ')' => Token::Close,
                '+' | '-' | '*' | '/' | '^' | '%' | '!' => Token::Op(c),
                '×' | '·' => Token::Op('*'),
                '÷' => Token::Op('/'),
                '−' | '–' => Token::Op('-'),
                _ => return None,
            });
            // ** is a power
            if c == '*' && chars.get(i + 1) == Some(&'*') {
                out.pop();
                out.push(Token::Op('^'));
                i += 1;
            }
            i += 1;
        }
    }
    Some(out)
}

/// A value and whether it was written as a percentage (`15%`), which
/// `+` and `-` read as a share of their left side.
type Value = (f64, bool);

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> { self.tokens.get(self.at) }

    fn eat(&mut self, token: &Token) -> bool {
        if self.peek() == Some(token) { self.at += 1; true } else { false }
    }

    fn expr(&mut self) -> Option<Value> {
        let mut left = self.term()?;
        loop {
            let sign = if self.eat(&Token::Op('+')) { 1.0 } else if self.eat(&Token::Op('-')) { -1.0 } else { break };
            let right = self.term()?;
            // 2400 + 15% → 2760, as Spotlight reads it
            let delta = if right.1 && !left.1 { left.0 * right.0 } else { right.0 };
            left = (left.0 + sign * delta, false);
        }
        Some(left)
    }

    fn term(&mut self) -> Option<Value> {
        let mut left = self.unary()?;
        loop {
            if self.eat(&Token::Op('*')) {
                let right = self.unary()?;
                left = (left.0 * right.0, false);
            } else if self.eat(&Token::Op('/')) {
                let right = self.unary()?;
                left = (left.0 / right.0, false);
            } else if self.eat(&Token::Ident("mod".into())) {
                let right = self.unary()?;
                left = (left.0.rem_euclid(right.0), false);
            } else if matches!(self.peek(), Some(Token::Open | Token::Ident(_))) && !matches!(self.peek(), Some(Token::Ident(w)) if w == "mod") {
                // implicit product: 2(3+4), 2pi
                let right = self.power()?;
                left = (left.0 * right.0, false);
            } else {
                break;
            }
        }
        Some(left)
    }

    /// A sign binds looser than a power: -2^2 is -(2^2).
    fn unary(&mut self) -> Option<Value> {
        if self.eat(&Token::Op('-')) {
            let (v, p) = self.unary()?;
            return Some((-v, p));
        }
        if self.eat(&Token::Op('+')) {
            return self.unary();
        }
        self.power()
    }

    fn power(&mut self) -> Option<Value> {
        let base = self.postfix()?;
        if self.eat(&Token::Op('^')) {
            // right-associative; the exponent may carry a sign (2^-1)
            let exponent = self.unary()?;
            return Some((base.0.powf(exponent.0), false));
        }
        Some(base)
    }

    fn postfix(&mut self) -> Option<Value> {
        let mut value = self.primary()?;
        loop {
            if self.eat(&Token::Op('%')) {
                value = (value.0 / 100.0, true);
            } else if self.eat(&Token::Op('!')) {
                let n = value.0;
                if n < 0.0 || n.fract() != 0.0 || n > 170.0 { return None; }
                value = ((1..=n as u64).map(|k| k as f64).product(), false);
            } else {
                return Some(value);
            }
        }
    }

    fn primary(&mut self) -> Option<Value> {
        match self.peek()?.clone() {
            Token::Num(n) => {
                self.at += 1;
                Some((n, false))
            }
            Token::Open => {
                self.at += 1;
                let value = self.expr()?;
                self.eat(&Token::Close).then_some(value)
            }
            Token::Ident(name) => {
                self.at += 1;
                match name.as_str() {
                    "pi" | "π" | "пи" => return Some((std::f64::consts::PI, false)),
                    "e" => return Some((std::f64::consts::E, false)),
                    _ => {}
                }
                let f: fn(f64) -> f64 = match name.as_str() {
                    "sqrt" | "корень" => f64::sqrt,
                    "cbrt" => f64::cbrt,
                    "sin" => f64::sin,
                    "cos" => f64::cos,
                    "tan" | "tg" => f64::tan,
                    "asin" => f64::asin,
                    "acos" => f64::acos,
                    "atan" => f64::atan,
                    "ln" => f64::ln,
                    "log" | "lg" => f64::log10,
                    "log2" => f64::log2,
                    "exp" => f64::exp,
                    "abs" => f64::abs,
                    "round" => f64::round,
                    "floor" => f64::floor,
                    "ceil" => f64::ceil,
                    _ => return None,
                };
                // sqrt 2 or sqrt(2)
                let (v, _) = self.power()?;
                Some((f(v), false))
            }
            _ => None,
        }
    }
}

/// `15% от 2400`, `15% of 2400`.
fn percent_of(query: &str) -> Option<f64> {
    let lower = query.to_lowercase();
    let (left, right) = lower.split_once(" от ").or_else(|| lower.split_once(" of "))?;
    let share = left.trim().strip_suffix('%')?;
    let eval = |s: &str| {
        let mut parser = Parser { tokens: tokenize(s)?, at: 0 };
        let v = parser.expr()?;
        (parser.at == parser.tokens.len()).then_some(v.0)
    };
    Some(eval(share)? / 100.0 * eval(right)?)
}

// ---- units ----------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dim {
    Length,
    Mass,
    Data,
    Time,
    Volume,
    Speed,
    Area,
}

/// (names, dimension, size in the dimension's base unit). Temperatures
/// are handled apart (their scales are offset).
const UNITS: &[(&[&str], Dim, f64)] = &[
    (&["mm", "мм", "миллиметр", "миллиметра", "миллиметров", "millimeter", "millimeters"], Dim::Length, 0.001),
    (&["cm", "см", "сантиметр", "сантиметра", "сантиметров", "centimeter", "centimeters"], Dim::Length, 0.01),
    (&["m", "м", "метр", "метра", "метров", "meter", "meters", "metre", "metres"], Dim::Length, 1.0),
    (&["km", "км", "километр", "километра", "километров", "kilometer", "kilometers"], Dim::Length, 1000.0),
    (&["in", "inch", "inches", "\"", "дюйм", "дюйма", "дюймов"], Dim::Length, 0.0254),
    (&["ft", "foot", "feet", "'", "фут", "фута", "футов"], Dim::Length, 0.3048),
    (&["yd", "yard", "yards", "ярд", "ярда", "ярдов"], Dim::Length, 0.9144),
    (&["mi", "mile", "miles", "миля", "мили", "миль", "милю"], Dim::Length, 1609.344),
    (&["nmi", "морская миля", "морских миль"], Dim::Length, 1852.0),
    (&["mg", "мг", "миллиграмм", "миллиграмма", "миллиграммов"], Dim::Mass, 0.001),
    (&["g", "г", "гр", "грамм", "грамма", "граммов", "gram", "grams"], Dim::Mass, 1.0),
    (&["kg", "кг", "килограмм", "килограмма", "килограммов", "kilogram", "kilograms"], Dim::Mass, 1000.0),
    (&["t", "т", "тонна", "тонны", "тонн", "ton", "tons", "tonne", "tonnes"], Dim::Mass, 1e6),
    (&["oz", "ounce", "ounces", "унция", "унции", "унций"], Dim::Mass, 28.349523125),
    (&["lb", "lbs", "pound", "pounds", "фунт", "фунта", "фунтов"], Dim::Mass, 453.59237),
    (&["b", "byte", "bytes", "байт", "байта", "байтов"], Dim::Data, 1.0),
    (&["kb", "кб", "килобайт", "килобайта", "килобайтов"], Dim::Data, 1e3),
    (&["mb", "мб", "мегабайт", "мегабайта", "мегабайтов"], Dim::Data, 1e6),
    (&["gb", "гб", "гигабайт", "гигабайта", "гигабайтов"], Dim::Data, 1e9),
    (&["tb", "тб", "терабайт", "терабайта", "терабайтов"], Dim::Data, 1e12),
    (&["kib", "киб"], Dim::Data, 1024.0),
    (&["mib", "миб"], Dim::Data, 1048576.0),
    (&["gib", "гиб"], Dim::Data, 1073741824.0),
    (&["tib", "тиб"], Dim::Data, 1099511627776.0),
    (&["ms", "мс", "миллисекунда", "миллисекунды", "миллисекунд"], Dim::Time, 0.001),
    (&["s", "sec", "secs", "second", "seconds", "с", "сек", "секунда", "секунды", "секунд", "секунду"], Dim::Time, 1.0),
    (&["min", "mins", "minute", "minutes", "мин", "минута", "минуты", "минут", "минуту"], Dim::Time, 60.0),
    (&["h", "hr", "hrs", "hour", "hours", "ч", "час", "часа", "часов"], Dim::Time, 3600.0),
    (&["d", "day", "days", "дн", "день", "дня", "дней", "сутки", "суток"], Dim::Time, 86400.0),
    (&["wk", "week", "weeks", "нед", "неделя", "недели", "недель", "неделю"], Dim::Time, 604800.0),
    (&["yr", "year", "years", "год", "года", "лет"], Dim::Time, 31557600.0),
    (&["ml", "мл", "миллилитр", "миллилитра", "миллилитров"], Dim::Volume, 0.001),
    (&["l", "л", "литр", "литра", "литров", "liter", "liters", "litre", "litres"], Dim::Volume, 1.0),
    (&["gal", "gallon", "gallons", "галлон", "галлона", "галлонов"], Dim::Volume, 3.785411784),
    (&["cup", "cups", "чашка", "чашки", "чашек"], Dim::Volume, 0.2365882365),
    (&["km/h", "kmh", "kph", "км/ч"], Dim::Speed, 1.0 / 3.6),
    (&["m/s", "м/с"], Dim::Speed, 1.0),
    (&["mph", "миль/ч"], Dim::Speed, 0.44704),
    (&["kn", "knot", "knots", "узел", "узла", "узлов"], Dim::Speed, 0.514444),
    (&["m2", "м2", "m²", "м²", "кв.м", "кв м"], Dim::Area, 1.0),
    (&["km2", "км2", "km²", "км²"], Dim::Area, 1e6),
    (&["ha", "га", "гектар", "гектара", "гектаров"], Dim::Area, 1e4),
    (&["acre", "acres", "акр", "акра", "акров"], Dim::Area, 4046.8564224),
    (&["sqft", "ft2", "ft²"], Dim::Area, 0.09290304),
];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Unit {
    Plain(Dim, f64),
    Celsius,
    Fahrenheit,
    Kelvin,
}

fn unit(name: &str) -> Option<Unit> {
    let name = name.trim().trim_end_matches('.').to_lowercase();
    match name.as_str() {
        "c" | "°c" | "℃" | "цельсий" | "цельсия" | "celsius" | "°с" | "с°" => return Some(Unit::Celsius),
        "f" | "°f" | "℉" | "фаренгейт" | "фаренгейта" | "fahrenheit" => return Some(Unit::Fahrenheit),
        "k" | "кельвин" | "кельвина" | "kelvin" => return Some(Unit::Kelvin),
        _ => {}
    }
    UNITS.iter().find(|(names, ..)| names.contains(&name.as_str())).map(|(_, dim, size)| Unit::Plain(*dim, *size))
}

/// Celsius degrees from `value` in `unit`, and back.
fn to_celsius(value: f64, unit: Unit) -> f64 {
    match unit {
        Unit::Fahrenheit => (value - 32.0) * 5.0 / 9.0,
        Unit::Kelvin => value - 273.15,
        _ => value,
    }
}

fn from_celsius(value: f64, unit: Unit) -> f64 {
    match unit {
        Unit::Fahrenheit => value * 9.0 / 5.0 + 32.0,
        Unit::Kelvin => value + 273.15,
        _ => value,
    }
}

/// A currency's ISO code from what people type.
fn currency(name: &str, rates: &Rates) -> Option<String> {
    let name = name.trim().trim_end_matches('.').to_lowercase();
    let code = match name.as_str() {
        "$" | "usd" | "доллар" | "доллара" | "долларов" | "доллары" | "бакс" | "баксов" | "dollar" | "dollars" => "USD",
        "€" | "eur" | "евро" | "euro" | "euros" => "EUR",
        "₽" | "rub" | "руб" | "рубль" | "рубля" | "рублей" | "рубли" | "р" => "RUB",
        "£" | "gbp" | "фунт стерлингов" | "фунтов стерлингов" => "GBP",
        "¥" | "jpy" | "иена" | "иены" | "иен" | "йена" | "йен" => "JPY",
        "cny" | "юань" | "юаня" | "юаней" | "rmb" => "CNY",
        "₺" | "try" | "лира" | "лиры" | "лир" => "TRY",
        "₸" | "kzt" | "тенге" => "KZT",
        "₴" | "uah" | "гривна" | "гривны" | "гривен" => "UAH",
        "byn" | "белорусский рубль" | "белорусских рублей" => "BYN",
        "amd" | "драм" | "драма" | "драмов" => "AMD",
        "gel" | "лари" => "GEL",
        "₿" | "btc" => "BTC",
        _ if name.len() == 3 && name.chars().all(|c| c.is_ascii_alphabetic()) => return {
            let code = name.to_uppercase();
            rates.rates.contains_key(&code).then_some(code)
        },
        _ => return None,
    };
    rates.rates.contains_key(code).then(|| code.to_string())
}

/// `<amount> <unit> в|to|in|-> <unit>`.
fn convert(query: &str, rates: Option<&Rates>) -> Option<Answer> {
    let lower = query.to_lowercase();
    let (left, target) = [" в ", " to ", " in ", " -> ", " → ", "->", "→", " = "].iter()
        .find_map(|sep| lower.rsplit_once(sep))?;
    let target = target.trim();
    // the amount is the leading expression, the unit what follows it
    // ("5 миль", "1.5gb", "$100", "100 km/h", "(2+3) кг")
    let left = left.trim();
    let (amount, from) = split_amount(left)?;
    let shown_target = target.to_string();
    if let (Some(from_u), Some(to_u)) = (unit(&from), unit(target)) {
        let value = match (from_u, to_u) {
            (Unit::Plain(a, size_a), Unit::Plain(b, size_b)) if a == b => amount * size_a / size_b,
            (Unit::Plain(..), _) | (_, Unit::Plain(..)) => return None,
            (a, b) => from_celsius(to_celsius(amount, a), b),
        };
        return Some(Answer { shown: format!("{} {shown_target}", shown(value)), copy: plain(value),
            detail: format!("{} {} → {shown_target}", shown(amount), from.trim()) });
    }
    let rates = rates?;
    let (a, b) = (currency(&from, rates)?, currency(target, rates)?);
    let value = amount / rates.rates[&a] * rates.rates[&b];
    let value = (value * 100.0).round() / 100.0;
    Some(Answer { shown: format!("{} {b}", shown(value)), copy: plain(value),
        detail: format!("{} {a} → {b} · курс на {}", shown(amount), rates.date) })
}

/// Split "5 миль" into (5, "миль"), "$100" into (100, "$").
fn split_amount(s: &str) -> Option<(f64, String)> {
    let eval = |text: &str| {
        let mut parser = Parser { tokens: tokenize(text)?, at: 0 };
        let v = parser.expr()?;
        (parser.at == parser.tokens.len() && v.0.is_finite()).then_some(v.0)
    };
    // a leading symbol: $100, €50
    if let Some(first) = s.chars().next().filter(|c| "$€£¥₽₺₸₴₿".contains(*c)) {
        return Some((eval(&s[first.len_utf8()..])?, first.to_string()));
    }
    // the unit starts after the last digit or closing bracket
    let cut = s.char_indices().filter(|(_, c)| c.is_ascii_digit() || *c == ')').map(|(i, c)| i + c.len_utf8()).last()?;
    let (amount, unit) = s.split_at(cut);
    let unit = unit.trim();
    if unit.is_empty() { return None; }
    Some((eval(amount)?, unit.to_string()))
}

// ---- numbers --------------------------------------------------------------

/// Ten significant digits, no trailing zeros.
fn rounded(value: f64) -> f64 {
    if value == 0.0 { return 0.0; }
    let digits = 10 - value.abs().log10().floor() as i32 - 1;
    if digits > 15 { return value; }
    let scale = 10f64.powi(digits.max(-300));
    (value * scale).round() / scale
}

/// For pasting: "1024", "8.04672", "1.5e-9".
fn plain(value: f64) -> String {
    let v = rounded(value);
    if v != 0.0 && (v.abs() >= 1e15 || v.abs() < 1e-6) {
        return format!("{v:e}");
    }
    let s = format!("{v:.10}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}

/// For reading: "1 024", "8,04672", "1,5 × 10⁻⁹".
fn shown(value: f64) -> String {
    let text = plain(value);
    if text.contains('e') {
        let (mantissa, exponent) = text.split_once('e').unwrap_or((&text, "0"));
        let superscript: String = exponent.chars().map(|c| match c {
            '-' => '⁻', '0' => '⁰', '1' => '¹', '2' => '²', '3' => '³', '4' => '⁴',
            '5' => '⁵', '6' => '⁶', '7' => '⁷', '8' => '⁸', _ => '⁹',
        }).collect();
        return format!("{} × 10{superscript}", mantissa.replace('.', ","));
    }
    let (int, frac) = text.split_once('.').map_or((text.as_str(), None), |(i, f)| (i, Some(f)));
    let (sign, digits) = int.strip_prefix('-').map_or(("", int), |d| ("-", d));
    // thousands apart by a narrow space once there are five digits
    let grouped = if digits.len() > 4 {
        let mut out = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 { out.push('\u{202F}'); }
            out.push(c);
        }
        out
    } else {
        digits.to_string()
    };
    match frac {
        Some(frac) => format!("{sign}{grouped},{frac}"),
        None => format!("{sign}{grouped}"),
    }
}

// ---- currency rates -------------------------------------------------------

#[derive(Debug)]
pub struct Rates {
    /// units of each currency per US dollar
    rates: HashMap<String, f64>,
    /// the day the rates are for, "5 окт"
    date: String,
    fetched: u64,
}

static RATES: OnceLock<Mutex<(Option<Arc<Rates>>, bool)>> = OnceLock::new();
const RATES_TTL: u64 = 12 * 3600;

fn rates_cell() -> &'static Mutex<(Option<Arc<Rates>>, bool)> {
    RATES.get_or_init(|| Mutex::new((None, false)))
}

fn rates() -> Option<Arc<Rates>> {
    rates_cell().lock().ok()?.0.clone()
}

fn cache_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("island-shell").join("rates.json"))
}

fn parse_rates(json: &str, fetched: u64) -> Option<Rates> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    if value.get("result").and_then(|r| r.as_str()) != Some("success") { return None; }
    let rates: HashMap<String, f64> = value.get("rates")?.as_object()?.iter()
        .filter_map(|(code, rate)| Some((code.clone(), rate.as_f64().filter(|r| *r > 0.0)?))).collect();
    let updated = value.get("time_last_update_unix").and_then(|t| t.as_u64()).unwrap_or(fetched);
    let date = crate::island::strftime_at("%-d.%m", updated as i64);
    (rates.contains_key("USD")).then_some(Rates { rates, date, fetched })
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Make sure the rates are loaded and fresh: reads the cache, and fetches
/// anew when it is older than 12 hours. Never blocks; call when the
/// launcher opens.
pub fn refresh_rates() {
    {
        let Ok(mut cell) = rates_cell().lock() else { return };
        let fresh = cell.0.as_ref().is_some_and(|r| now_secs().saturating_sub(r.fetched) < RATES_TTL);
        if fresh || cell.1 { return; }
        cell.1 = true;
    }
    std::thread::spawn(|| {
        let path = cache_path();
        let cached = path.as_ref().and_then(|p| {
            let modified = p.metadata().ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
            parse_rates(&std::fs::read_to_string(p).ok()?, modified)
        });
        let stale = cached.as_ref().is_none_or(|r| now_secs().saturating_sub(r.fetched) >= RATES_TTL);
        if let Some(rates) = cached {
            if let Ok(mut cell) = rates_cell().lock() { cell.0 = Some(Arc::new(rates)); }
        }
        if stale {
            let fetched = std::process::Command::new("curl")
                .args(["-fsS", "--max-time", "8", "https://open.er-api.com/v6/latest/USD"])
                .stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()
                .filter(|out| out.status.success())
                .and_then(|out| String::from_utf8(out.stdout).ok());
            if let Some(json) = fetched {
                if let Some(rates) = parse_rates(&json, now_secs()) {
                    if let Some(path) = &path {
                        let _ = std::fs::create_dir_all(path.parent().unwrap_or(path));
                        let _ = std::fs::write(path, &json);
                    }
                    if let Ok(mut cell) = rates_cell().lock() { cell.0 = Some(Arc::new(rates)); }
                }
            }
        }
        // a failed fetch is retried on a later opening, not in a loop
        if let Ok(mut cell) = rates_cell().lock() { cell.1 = false; }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calc(q: &str) -> Option<String> { answer(q).map(|a| a.copy) }

    #[test]
    fn arithmetic() {
        assert_eq!(calc("2*(17+5)").as_deref(), Some("44"));
        assert_eq!(calc("2^10").as_deref(), Some("1024"));
        assert_eq!(calc("2**3**2").as_deref(), Some("512"));
        assert_eq!(calc("-2^2").as_deref(), Some("-4"));
        assert_eq!(calc("1,5 + 2,25").as_deref(), Some("3.75"));
        assert_eq!(calc("10/4").as_deref(), Some("2.5"));
        assert_eq!(calc("2400 + 15%").as_deref(), Some("2760"));
        assert_eq!(calc("2400 - 15%").as_deref(), Some("2040"));
        assert_eq!(calc("15% от 2400").as_deref(), Some("360"));
        assert_eq!(calc("50% * 30").as_deref(), Some("15"));
        assert_eq!(calc("sqrt 2").as_deref(), Some("1.414213562"));
        assert_eq!(calc("2pi").as_deref(), Some("6.283185307"));
        assert_eq!(calc("5!").as_deref(), Some("120"));
        assert_eq!(calc("7 mod 3").as_deref(), Some("1"));
        assert_eq!(calc("0.1+0.2").as_deref(), Some("0.3"));
        assert_eq!(calc("1e6*3").as_deref(), Some("3000000"));
        // not questions
        assert_eq!(calc("2048"), None);
        assert_eq!(calc("firefox"), None);
        assert_eq!(calc("7-zip"), None);
        assert_eq!(calc("pi"), None);
        assert_eq!(calc("1/0"), None);
        assert_eq!(calc("(2+3"), None);
    }

    #[test]
    fn conversions() {
        assert_eq!(answer("5 миль в км").map(|a| a.shown).as_deref(), Some("8,04672 км"));
        assert_eq!(calc("100 f to c").as_deref(), Some("37.77777778"));
        assert_eq!(calc("0 c в k").as_deref(), Some("273.15"));
        assert_eq!(calc("1.5 GB в MiB").as_deref(), Some("1430.511475"));
        assert_eq!(calc("90 км/ч в м/с").as_deref(), Some("25"));
        assert_eq!(calc("2 часа в минуты").as_deref(), Some("120"));
        assert_eq!(calc("12:30"), None, "a time is not a division");
        assert_eq!(calc("2 часа в мин").as_deref(), Some("120"));
        assert_eq!(calc("3 кг в км"), None, "different dimensions");
        assert_eq!(answer("12345678 m в mm").map(|a| a.shown).as_deref(), Some("12\u{202F}345\u{202F}678\u{202F}000 mm"));
        let rates = Rates { rates: [("USD".to_string(), 1.0), ("RUB".to_string(), 90.0), ("EUR".to_string(), 0.9)].into(), date: "5.10".into(), fetched: 0 };
        assert_eq!(convert("100 usd в rub", Some(&rates)).map(|a| a.copy).as_deref(), Some("9000"));
        assert_eq!(convert("$100 в евро", Some(&rates)).map(|a| a.shown).as_deref(), Some("90 EUR"));
        assert_eq!(convert("100 usd в rub", None), None);
    }

    #[test]
    fn numbers_read_well() {
        assert_eq!(shown(1234.5), "1234,5");
        assert_eq!(shown(-12345.0), "-12\u{202F}345");
        assert_eq!(shown(1.5e-9), "1,5 × 10⁻⁹");
        assert_eq!(plain(-0.0), "0");
    }
}
