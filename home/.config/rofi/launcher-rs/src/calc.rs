use std::f64::consts::{E, PI, TAU};

#[derive(Debug, Clone, PartialEq)]
pub struct CalcOutput {
    pub formatted: String,
    pub value: f64,
}

#[derive(Debug, Clone, Copy)]
struct Val {
    v: f64,
    is_pct: bool,
}

impl Val {
    fn plain(v: f64) -> Self {
        Self { v, is_pct: false }
    }
    fn pct(v: f64) -> Self {
        Self { v, is_pct: true }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Plus,
    Minus,
    Mul,
    Div,
    Pow,
    ModOrPct,
    Bang,
    LParen,
    RParen,
    Comma,
}

impl Tok {
    fn ends_value(&self) -> bool {
        matches!(self, Tok::Num(_) | Tok::RParen | Tok::Bang)
    }

    fn is_constant_ident(s: &str) -> bool {
        matches!(s, "pi" | "e" | "tau" | "phi")
    }
}

fn is_multi_arg_func(name: &str) -> bool {
    matches!(name, "pow" | "min" | "max" | "log" | "atan2")
}

fn tokenize(input: &str) -> Result<Vec<Tok>, String> {
    let normalized: String = input
        .chars()
        .map(|c| match c {
            '×' | '⋅' => '*',
            '÷' | '∶' => '/',
            '−' | '–' | '—' => '-',
            'π' => 'π', // handled below
            other => other,
        })
        .collect();

    let chars: Vec<char> = normalized.chars().collect();
    let mut raw: Vec<Tok> = Vec::new();
    let mut i = 0;

    // Track paren stack: true if opened immediately after a multi-arg function name.
    let mut paren_stack: Vec<bool> = Vec::new();

    while i < chars.len() {
        let ch = chars[i];
        if ch.is_whitespace() {
            i += 1;
            continue;
        }

        if ch == 'π' {
            raw.push(Tok::Ident("pi".to_string()));
            i += 1;
            continue;
        }
        if ch == 'τ' {
            raw.push(Tok::Ident("tau".to_string()));
            i += 1;
            continue;
        }

        let in_multi_arg_call = paren_stack.last().copied().unwrap_or(false);
        let starts_num = ch.is_ascii_digit()
            || (ch == '.' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit())
            || (!in_multi_arg_call
                && ch == ','
                && i + 1 < chars.len()
                && chars[i + 1].is_ascii_digit()
                && !matches!(raw.last(), Some(Tok::Num(_) | Tok::RParen | Tok::Ident(_))));

        if starts_num {
            let mut s = String::new();
            let mut seen_dot = false;
            while i < chars.len() {
                let c = chars[i];
                if c.is_ascii_digit() {
                    s.push(c);
                    i += 1;
                } else if c == '.' && !seen_dot {
                    seen_dot = true;
                    s.push('.');
                    i += 1;
                } else if c == ','
                    && !seen_dot
                    && !in_multi_arg_call
                    && i + 1 < chars.len()
                    && chars[i + 1].is_ascii_digit()
                {
                    seen_dot = true;
                    s.push('.');
                    i += 1;
                } else if c == '_' {
                    i += 1;
                } else {
                    break;
                }
            }
            // Scientific notation e.g. 1e-5, 2.5E6
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let mut look = i + 1;
                if look < chars.len() && (chars[look] == '+' || chars[look] == '-') {
                    look += 1;
                }
                if look < chars.len() && chars[look].is_ascii_digit() {
                    s.push('e');
                    i += 1;
                    if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
                        s.push(chars[i]);
                        i += 1;
                    }
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        s.push(chars[i]);
                        i += 1;
                    }
                }
            }
            if s.starts_with('.') {
                s.insert(0, '0');
            }
            let n: f64 = s
                .parse()
                .map_err(|_| format!("Некорректное число: {s}"))?;
            raw.push(Tok::Num(n));
            continue;
        }

        if ch.is_alphabetic() || ch == '_' {
            let mut name = String::new();
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                name.push(chars[i].to_ascii_lowercase());
                i += 1;
            }
            if name == "mod" {
                raw.push(Tok::ModOrPct);
            } else {
                raw.push(Tok::Ident(name));
            }
            continue;
        }

        match ch {
            '+' => {
                raw.push(Tok::Plus);
                i += 1;
            }
            '-' => {
                raw.push(Tok::Minus);
                i += 1;
            }
            '*' => {
                if i + 1 < chars.len() && chars[i + 1] == '*' {
                    raw.push(Tok::Pow);
                    i += 2;
                } else {
                    raw.push(Tok::Mul);
                    i += 1;
                }
            }
            '/' | ':' => {
                raw.push(Tok::Div);
                i += 1;
            }
            '^' => {
                raw.push(Tok::Pow);
                i += 1;
            }
            '%' => {
                raw.push(Tok::ModOrPct);
                i += 1;
            }
            '!' => {
                raw.push(Tok::Bang);
                i += 1;
            }
            '(' => {
                let is_multi = matches!(raw.last(), Some(Tok::Ident(id)) if is_multi_arg_func(id));
                paren_stack.push(is_multi);
                raw.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                paren_stack.pop();
                raw.push(Tok::RParen);
                i += 1;
            }
            ',' => {
                raw.push(Tok::Comma);
                i += 1;
            }
            other => {
                return Err(format!("Неизвестный символ «{other}»"));
            }
        }
    }

    // Insert implicit multiplication where mathematically natural:
    // 2pi, 2(3+4), (1+2)(3+4), (2)pi, 2sqrt(9), 5!2, 5!(2)
    let mut out = Vec::with_capacity(raw.len() + 4);
    for idx in 0..raw.len() {
        let cur = &raw[idx];
        if let Some(prev) = out.last() {
            let prev_val = match prev {
                Tok::Ident(name) => Tok::is_constant_ident(name),
                t => t.ends_value(),
            };
            let next_starts_val = match cur {
                Tok::Num(_) | Tok::Ident(_) => true,
                Tok::LParen => !matches!(prev, Tok::Ident(name) if !Tok::is_constant_ident(name)),
                _ => false,
            };
            if prev_val && next_starts_val {
                out.push(Tok::Mul);
            }
        }
        out.push(cur.clone());
    }

    Ok(out)
}

struct Parser<'a> {
    toks: &'a [Tok],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<&Tok> {
        let t = self.toks.get(self.pos);
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn parse_expr(&mut self) -> Result<Val, String> {
        let mut acc = self.parse_term()?;
        while let Some(op) = self.peek().cloned() {
            match op {
                Tok::Plus => {
                    self.next();
                    let rhs = self.parse_term()?;
                    if !acc.is_pct && rhs.is_pct {
                        acc = Val::plain(acc.v * (1.0 + rhs.v));
                    } else {
                        acc = Val::plain(acc.v + rhs.v);
                    }
                }
                Tok::Minus => {
                    self.next();
                    let rhs = self.parse_term()?;
                    if !acc.is_pct && rhs.is_pct {
                        acc = Val::plain(acc.v * (1.0 - rhs.v));
                    } else {
                        acc = Val::plain(acc.v - rhs.v);
                    }
                }
                _ => break,
            }
        }
        Ok(acc)
    }

    fn starts_factor(&self, offset: usize) -> bool {
        matches!(
            self.toks.get(self.pos + offset),
            Some(
                Tok::Num(_)
                    | Tok::Ident(_)
                    | Tok::LParen
                    | Tok::Plus
                    | Tok::Minus
            )
        )
    }

    fn parse_term(&mut self) -> Result<Val, String> {
        let mut acc = self.parse_unary()?;
        while let Some(op) = self.peek().cloned() {
            match op {
                Tok::Mul => {
                    self.next();
                    let rhs = self.parse_unary()?;
                    acc = Val::plain(acc.v * rhs.v);
                }
                Tok::Div => {
                    self.next();
                    let rhs = self.parse_unary()?;
                    if rhs.v == 0.0 {
                        return Err("Деление на ноль".to_string());
                    }
                    acc = Val::plain(acc.v / rhs.v);
                }
                Tok::ModOrPct if self.starts_factor(1) => {
                    self.next();
                    let rhs = self.parse_unary()?;
                    if rhs.v == 0.0 {
                        return Err("Деление на ноль".to_string());
                    }
                    acc = Val::plain(acc.v % rhs.v);
                }
                _ => break,
            }
        }
        Ok(acc)
    }

    fn parse_unary(&mut self) -> Result<Val, String> {
        match self.peek() {
            Some(Tok::Plus) => {
                self.next();
                self.parse_unary()
            }
            Some(Tok::Minus) => {
                self.next();
                let inner = self.parse_unary()?;
                Ok(Val {
                    v: -inner.v,
                    is_pct: inner.is_pct,
                })
            }
            _ => self.parse_power(),
        }
    }

    fn parse_power(&mut self) -> Result<Val, String> {
        let base = self.parse_postfix()?;
        if matches!(self.peek(), Some(Tok::Pow)) {
            self.next();
            let exp = self.parse_unary()?;
            let res = base.v.powf(exp.v);
            if !res.is_finite() {
                return Err("Результат вне диапазона".to_string());
            }
            Ok(Val::plain(res))
        } else {
            Ok(base)
        }
    }

    fn parse_postfix(&mut self) -> Result<Val, String> {
        let mut val = self.parse_primary()?;
        loop {
            match self.peek() {
                Some(Tok::ModOrPct) if !self.starts_factor(1) => {
                    self.next();
                    val = Val::pct(val.v / 100.0);
                }
                Some(Tok::Bang) => {
                    self.next();
                    val = Val::plain(factorial(val.v)?);
                }
                _ => break,
            }
        }
        Ok(val)
    }

    fn parse_primary(&mut self) -> Result<Val, String> {
        match self.next().cloned() {
            Some(Tok::Num(n)) => Ok(Val::plain(n)),
            Some(Tok::LParen) => {
                let inner = self.parse_expr()?;
                match self.next() {
                    Some(Tok::RParen) => Ok(Val::plain(inner.v)),
                    _ => Err("Не хватает закрывающей скобки )".to_string()),
                }
            }
            Some(Tok::Ident(name)) => {
                if matches!(self.peek(), Some(Tok::LParen)) {
                    self.next(); // consume '('
                    let mut args = Vec::new();
                    if !matches!(self.peek(), Some(Tok::RParen)) {
                        loop {
                            args.push(self.parse_expr()?.v);
                            if matches!(self.peek(), Some(Tok::Comma)) {
                                self.next();
                            } else {
                                break;
                            }
                        }
                    }
                    match self.next() {
                        Some(Tok::RParen) => {}
                        _ => return Err("Не хватает ) после аргументов функции".to_string()),
                    }
                    eval_func(&name, &args).map(Val::plain)
                } else {
                    match name.as_str() {
                        "pi" => Ok(Val::plain(PI)),
                        "e" => Ok(Val::plain(E)),
                        "tau" => Ok(Val::plain(TAU)),
                        "phi" => Ok(Val::plain((1.0 + 5.0_f64.sqrt()) * 0.5)),
                        _ => Err(format!("Неизвестный идентификатор «{name}»")),
                    }
                }
            }
            Some(other) => Err(format!("Ожидалось число или выражение, а не {other:?}")),
            None => Err("Незавершённое выражение".to_string()),
        }
    }
}

fn factorial(n: f64) -> Result<f64, String> {
    if n < 0.0 || (n - n.round()).abs() > 1e-12 {
        return Err("Факториал определён только для целых ≥ 0".to_string());
    }
    let k = n.round() as u64;
    if k > 170 {
        return Err("Слишком большой факториал (> 170!)".to_string());
    }
    let mut acc = 1.0_f64;
    for m in 2..=k {
        acc *= m as f64;
    }
    Ok(acc)
}

fn eval_func(name: &str, args: &[f64]) -> Result<f64, String> {
    let check1 = |a: &[f64]| -> Result<f64, String> {
        if a.len() == 1 {
            Ok(a[0])
        } else {
            Err(format!("Функция {name}() принимает 1 аргумент"))
        }
    };
    let check2 = |a: &[f64]| -> Result<(f64, f64), String> {
        if a.len() == 2 {
            Ok((a[0], a[1]))
        } else {
            Err(format!("Функция {name}() принимает 2 аргумента"))
        }
    };

    let res = match name {
        "sqrt" => {
            let x = check1(args)?;
            if x < 0.0 {
                return Err("Корень из отрицательного числа".to_string());
            }
            x.sqrt()
        }
        "cbrt" => check1(args)?.cbrt(),
        "abs" => check1(args)?.abs(),
        "sin" => check1(args)?.sin(),
        "cos" => check1(args)?.cos(),
        "tan" => check1(args)?.tan(),
        "asin" => check1(args)?.asin(),
        "acos" => check1(args)?.acos(),
        "atan" => check1(args)?.atan(),
        "atan2" => {
            let (y, x) = check2(args)?;
            y.atan2(x)
        }
        "ln" => {
            let x = check1(args)?;
            if x <= 0.0 {
                return Err("Логарифм определён для чисел > 0".to_string());
            }
            x.ln()
        }
        "log" => match args.len() {
            1 => {
                if args[0] <= 0.0 {
                    return Err("Логарифм определён для чисел > 0".to_string());
                }
                args[0].log10()
            }
            2 => {
                if args[0] <= 0.0 || args[1] <= 0.0 || (args[1] - 1.0).abs() < 1e-12 {
                    return Err("Некорректное основание логарифма".to_string());
                }
                args[0].log(args[1])
            }
            _ => return Err("Функция log() принимает 1 или 2 аргумента".to_string()),
        },
        "log10" => {
            let x = check1(args)?;
            if x <= 0.0 {
                return Err("Логарифм определён для чисел > 0".to_string());
            }
            x.log10()
        }
        "log2" => {
            let x = check1(args)?;
            if x <= 0.0 {
                return Err("Логарифм определён для чисел > 0".to_string());
            }
            x.log2()
        }
        "exp" => check1(args)?.exp(),
        "floor" => check1(args)?.floor(),
        "ceil" => check1(args)?.ceil(),
        "round" => check1(args)?.round(),
        "pow" => {
            let (b, e) = check2(args)?;
            b.powf(e)
        }
        "min" => {
            if args.is_empty() {
                return Err("Функция min() требует аргументы".to_string());
            }
            args.iter().copied().fold(f64::INFINITY, f64::min)
        }
        "max" => {
            if args.is_empty() {
                return Err("Функция max() требует аргументы".to_string());
            }
            args.iter().copied().fold(f64::NEG_INFINITY, f64::max)
        }
        "fact" => factorial(check1(args)?)?,
        _ => return Err(format!("Неизвестная функция «{name}»")),
    };

    if !res.is_finite() {
        return Err("Результат не является конечным числом".to_string());
    }
    Ok(res)
}

pub fn format_number(val: f64) -> String {
    if !val.is_finite() {
        return "Ошибка".to_string();
    }
    let clean = if val.abs() < 1e-12 { 0.0 } else { val };

    // Clean 12 significant digits using scientific normalization first.
    if clean == 0.0 {
        return "0".to_string();
    }
    if clean.abs() >= 1e12 || clean.abs() < 1e-7 {
        let sci = format!("{clean:.11e}");
        if let Some((mant, exp)) = sci.split_once('e') {
            let trimmed = mant.trim_end_matches('0').trim_end_matches('.');
            return format!("{trimmed}e{exp}");
        }
        return sci;
    }

    let rounded: f64 = format!("{clean:.11e}").parse().unwrap_or(clean);
    let rounded = if (rounded - rounded.round()).abs() < 1e-11 && rounded.abs() < 1e12 {
        rounded.round()
    } else {
        rounded
    };

    if rounded.fract() == 0.0 {
        format!("{:.0}", rounded)
    } else {
        let decimals = (11 - rounded.abs().log10().floor().max(0.0) as isize).clamp(0, 11) as usize;
        let s = format!("{rounded:.decimals$}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

pub fn evaluate_expr(expr: &str) -> Result<CalcOutput, String> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return Err("Введи выражение после «=»".to_string());
    }
    let toks = tokenize(trimmed)?;
    if toks.is_empty() {
        return Err("Введи выражение после «=»".to_string());
    }
    let mut parser = Parser { toks: &toks, pos: 0 };
    let val = parser.parse_expr()?;
    if parser.pos < parser.toks.len() {
        return Err(format!("Лишний фрагмент в выражении"));
    }
    if !val.v.is_finite() {
        return Err("Результат вне диапазона".to_string());
    }
    Ok(CalcOutput {
        formatted: format_number(val.v),
        value: val.v,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(expr: &str) -> String {
        evaluate_expr(expr).expect(expr).formatted
    }

    #[test]
    fn arithmetic_precedence_and_parens() {
        assert_eq!(ok("2 + 3 * 4"), "14");
        assert_eq!(ok("(2 + 3) * 4"), "20");
        assert_eq!(ok("2^3^2"), "512");
        assert_eq!(ok("2**10"), "1024");
        assert_eq!(ok("-2^2"), "-4");
        assert_eq!(ok("(-2)^2"), "4");
        assert_eq!(ok("10 - 3 - 2"), "5");
        assert_eq!(ok("100 % 7"), "2");
        assert_eq!(ok("17 mod 5"), "2");
    }

    #[test]
    fn percentages_and_comma_decimals() {
        assert_eq!(ok("50%"), "0.5");
        assert_eq!(ok("15% * 200"), "30");
        assert_eq!(ok("200 + 15%"), "230");
        assert_eq!(ok("200 - 15%"), "170");
        assert_eq!(ok("2,5 * 4"), "10");
        assert_eq!(ok("sqrt(2,25)"), "1.5");
        assert_eq!(ok("0.1 + 0.2"), "0.3");
    }

    #[test]
    fn functions_constants_factorial_and_implicit_mul() {
        assert_eq!(ok("5!"), "120");
        assert_eq!(ok("0!"), "1");
        assert_eq!(ok("sqrt(144) + cbrt(27)"), "15");
        assert_eq!(ok("sin(pi)"), "0");
        assert_eq!(ok("cos(pi)"), "-1");
        assert_eq!(ok("ln(e^3)"), "3");
        assert_eq!(ok("log(1000)"), "3");
        assert_eq!(ok("log2(256)"), "8");
        assert_eq!(ok("pow(2, 5)"), "32");
        assert_eq!(ok("min(9, 4, 7)"), "4");
        assert_eq!(ok("max(3, 12, 8)"), "12");
        assert_eq!(ok("2(3 + 4)"), "14");
        assert_eq!(ok("2pi"), "6.28318530718");
    }

    #[test]
    fn handles_errors_gracefully() {
        assert!(evaluate_expr("10 / 0").is_err());
        assert!(evaluate_expr("sqrt(-4)").is_err());
        assert!(evaluate_expr("(2 + 3").is_err());
        assert!(evaluate_expr("10 % 0").is_err());
        assert!(evaluate_expr("(-1)!").is_err());
        assert!(evaluate_expr("   ").is_err());
    }
}
