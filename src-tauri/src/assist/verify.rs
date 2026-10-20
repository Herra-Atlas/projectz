//! Arithmetic the app can answer without a model.
//!
//! # Why this exists
//!
//! `candidate.rs` can already tell an arithmetic block from everything else with more
//! confidence than any other shape, because a sum has a grammar: digits, an operator and,
//! usually, an `=`. That is the one case where the answer is not a judgement -- it is a
//! computation -- so it should not be spent on a model at all, least of all a small one.
//!
//! A local 2B will get `14 - 8` right most of the time and `3x + 7 = 22` wrong some of the
//! time, and the user has no way to tell which card was which. Solving it here makes that
//! class of card exact, instant and free. It also means assist can answer a page of sums
//! with no model chosen, no request spent and no wait.
//!
//! # Why it is deliberately narrow
//!
//! A solver that guesses is worse than one that declines: `SCORE 0 / 4` is a counter, not a
//! division, and `s = "abc"` is an assignment, not an equation. So the whole text has to be
//! the expression (after a small closed set of leading phrases and trailing punctuation),
//! and anything else is `None`. The gate has already classified the block as arithmetic
//! before this is called, so the input is expected to be one.
//!
//! # Why exact arithmetic
//!
//! `1/2 + 1/3` is `5/6`, not `0.8333333`. Floats would also make a correct answer look
//! wrong on a comparison, and a card that says `6.000000000000001` is worse than no card.
//! Everything is a reduced rational internally, and only the rendering chooses between an
//! integer, a terminating decimal and a fraction.

/// A reduced rational number. `den` is always positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rational {
    num: i128,
    den: i128,
}

impl Rational {
    const ONE: Self = Self { num: 1, den: 1 };

    fn new(num: i128, den: i128) -> Option<Self> {
        if den == 0 {
            return None;
        }
        let (num, den) = if den < 0 {
            (num.checked_neg()?, den.checked_neg()?)
        } else {
            (num, den)
        };
        let divisor = gcd(num.unsigned_abs(), den.unsigned_abs()) as i128;
        Some(Self {
            num: num / divisor,
            den: den / divisor,
        })
    }

    fn zero() -> Self {
        Self { num: 0, den: 1 }
    }

    fn is_zero(self) -> bool {
        self.num == 0
    }

    fn add(self, other: Self) -> Option<Self> {
        Self::new(
            self.num
                .checked_mul(other.den)?
                .checked_add(other.num.checked_mul(self.den)?)?,
            self.den.checked_mul(other.den)?,
        )
    }

    fn sub(self, other: Self) -> Option<Self> {
        Self::new(
            self.num
                .checked_mul(other.den)?
                .checked_sub(other.num.checked_mul(self.den)?)?,
            self.den.checked_mul(other.den)?,
        )
    }

    fn mul(self, other: Self) -> Option<Self> {
        Self::new(
            self.num.checked_mul(other.num)?,
            self.den.checked_mul(other.den)?,
        )
    }

    fn div(self, other: Self) -> Option<Self> {
        Self::new(
            self.num.checked_mul(other.den)?,
            self.den.checked_mul(other.num)?,
        )
    }

    /// Raises to an integer power. Anything else is refused rather than approximated.
    fn pow(self, exponent: i128) -> Option<Self> {
        if exponent.unsigned_abs() > 20 {
            return None;
        }
        let power = exponent.unsigned_abs() as u32;
        if exponent < 0 {
            Self::new(self.den.checked_pow(power)?, self.num.checked_pow(power)?)
        } else {
            Self::new(self.num.checked_pow(power)?, self.den.checked_pow(power)?)
        }
    }

    /// The shortest honest spelling: whole number, terminating decimal, or fraction.
    fn render(self) -> String {
        if self.den == 1 {
            return self.num.to_string();
        }
        // A denominator with no prime factor but 2 and 5 terminates. Anything else is
        // written as a fraction, because a decimal would have to be rounded and a rounded
        // answer on someone's screen is a wrong answer.
        let mut rest = self.den;
        for factor in [2i128, 5] {
            while rest % factor == 0 {
                rest /= factor;
            }
        }
        if rest != 1 {
            return format!("{}/{}", self.num, self.den);
        }
        let negative = self.num < 0;
        let mut remainder = self.num.unsigned_abs();
        let denominator = self.den as u128;
        let whole = remainder / denominator;
        remainder %= denominator;
        let mut out = String::new();
        if negative {
            out.push('-');
        }
        out.push_str(&whole.to_string());
        if remainder != 0 {
            out.push('.');
            for _ in 0..10 {
                if remainder > u128::MAX / 10 {
                    break;
                }
                remainder *= 10;
                out.push(char::from(b'0' + (remainder / denominator) as u8));
                remainder %= denominator;
                if remainder == 0 {
                    break;
                }
            }
        }
        out
    }
}

fn gcd(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let rest = left % right;
        left = right;
        right = rest;
    }
    left.max(1)
}

/// One token of an arithmetic expression.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Token {
    Number(Rational),
    /// The unknown in an equation. Never present in an expression that is only computed.
    X,
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Percent,
    Open,
    Close,
}

/// Answers a block that is a sum, a percentage or a one-variable linear equation.
///
/// `None` means "this is not a self-contained arithmetic question", which is the safe
/// answer: the caller falls back to the model, which may still decline.
pub fn solve(text: &str) -> Option<String> {
    let lowered = text.to_lowercase();
    let stripped = strip_leading(&lowered);
    let normalized = normalize(stripped);
    let expression = normalized
        .trim()
        .trim_end_matches(['?', '.', '!', ';', ':'])
        .trim();
    if expression.is_empty() {
        return None;
    }
    let parts: Vec<&str> = expression.split('=').collect();
    match parts.as_slice() {
        [left, right] => {
            let left = left.trim();
            let right = right.trim();
            if left.is_empty() {
                return None;
            }
            // An unfinished sum -- `14 - 8 =` -- is the question.
            if right.is_empty() || right == "?" {
                return compute(left);
            }
            // An equation with one unknown. A complete numeric equation (`14 - 8 = 6`)
            // already contains its answer, so it is a statement rather than a question.
            if left.contains('x') && !right.contains('x') {
                return solve_linear(left, right);
            }
            None
        }
        // No equals sign: a bare sum or a fraction. The gate has already decided it is
        // arithmetic, so this is computed rather than sent on.
        [single] => {
            if single.contains('x') {
                return None;
            }
            compute(single)
        }
        _ => None,
    }
}

/// Closed list of phrasings that mean the same as "compute this".
///
/// Longest first, so "what is the value of" is consumed whole rather than leaving "the
/// value of" behind for a second pass.
const PREFIXES: [&str; 11] = [
    "what is the value of",
    "what's the value of",
    "what is",
    "what's",
    "whats",
    "how much is",
    "calculate",
    "compute",
    "evaluate",
    "solve for x",
    "solve",
];

fn strip_leading(text: &str) -> &str {
    let mut rest = text.trim_start();
    loop {
        rest = rest.trim_start_matches([':', '=', ' ', '\t']);
        let mut matched = false;
        for prefix in PREFIXES {
            if let Some(after) = rest.strip_prefix(prefix) {
                // The prefix must be a word, not the start of one: `solvex` is not `solve`.
                let word_boundary = after
                    .chars()
                    .next()
                    .map(|character| !character.is_alphanumeric())
                    .unwrap_or(true);
                if word_boundary {
                    rest = after;
                    matched = true;
                    break;
                }
            }
        }
        if !matched {
            return rest;
        }
    }
}

/// Folds the ways a person writes the same operator into symbols the tokenizer knows.
fn normalize(text: &str) -> String {
    let mut text = text.to_string();
    for (from, to) in [
        ("multiplied by", "*"),
        ("divided by", "/"),
        ("squared", "^2"),
        ("cubed", "^3"),
        ("plus", "+"),
        ("minus", "-"),
        ("times", "*"),
    ] {
        text = text.replace(from, to);
    }
    text = text.replace("percent", "%");
    // `15% of 240` is `15% * 240`; the postfix percent is what turns it into 0.15.
    text = text.replace("% of", "% *");
    text = text
        .replace('×', "*")
        .replace('·', "*")
        .replace('⋅', "*")
        .replace('÷', "/")
        .replace('−', "-")
        .replace('–', "-")
        .replace('—', "-")
        // Thousands separators, which people write in sums and never mean as separators.
        .replace(',', "");
    text
}

fn compute(expression: &str) -> Option<String> {
    let tokens = tokenize(expression)?;
    if !has_binary_operator(&tokens) {
        return None;
    }
    let value = eval(&tokens, Rational::zero())?;
    Some(value.render())
}

fn solve_linear(left: &str, right: &str) -> Option<String> {
    let tokens = tokenize(left)?;
    if !tokens.contains(&Token::X) {
        return None;
    }
    // Three samples decide both the slope and whether the expression is linear at all.
    // x^2 fits the first two samples and fails the third, which is the whole check.
    let at_zero = eval(&tokens, Rational::zero())?;
    let at_one = eval(&tokens, Rational::ONE)?;
    let at_two = eval(&tokens, Rational::new(2, 1)?)?;
    let slope = at_one.sub(at_zero)?;
    if slope.is_zero() {
        return None;
    }
    let predicted = slope.mul(Rational::new(2, 1)?)?.add(at_zero)?;
    if at_two != predicted {
        return None;
    }
    let right_tokens = tokenize(right)?;
    if right_tokens.contains(&Token::X) {
        return None;
    }
    let target = eval(&right_tokens, Rational::zero())?;
    let value = target.sub(at_zero)?.div(slope)?;
    Some(format!("x = {}", value.render()))
}

/// Whether there is at least one operator joining two things.
///
/// A leading sign is not one: `-5` is a number, and carding it would be noise.
fn has_binary_operator(tokens: &[Token]) -> bool {
    tokens
        .iter()
        .enumerate()
        .any(|(position, token)| match token {
            Token::Plus | Token::Star | Token::Slash | Token::Caret | Token::Percent => true,
            Token::Minus => position > 0,
            _ => false,
        })
}

fn tokenize(text: &str) -> Option<Vec<Token>> {
    let characters: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut at = 0;
    while at < characters.len() {
        let character = characters[at];
        if character.is_whitespace() {
            at += 1;
            continue;
        }
        if character.is_ascii_digit()
            || (character == '.'
                && characters
                    .get(at + 1)
                    .is_some_and(|next| next.is_ascii_digit()))
        {
            let start = at;
            let mut seen_dot = false;
            while at < characters.len() {
                let current = characters[at];
                if current.is_ascii_digit() {
                    at += 1;
                } else if current == '.' && !seen_dot {
                    seen_dot = true;
                    at += 1;
                } else {
                    break;
                }
            }
            let literal: String = characters[start..at].iter().collect();
            // Exact from the decimal string rather than from a float: a value that goes
            // through `f64` comes back as something the user never wrote.
            let (digits, scale) = match literal.split_once('.') {
                Some((whole, fraction)) => (format!("{whole}{fraction}"), fraction.len()),
                None => (literal.clone(), 0),
            };
            let num = digits.parse::<i128>().ok()?;
            let den = 10i128.checked_pow(scale as u32)?;
            tokens.push(Token::Number(Rational::new(num, den)?));
            if peek(&characters, at).is_some_and(|next| next == 'x' || next == '(') {
                tokens.push(Token::Star);
            }
            continue;
        }
        match character {
            'x' => {
                tokens.push(Token::X);
                if peek(&characters, at + 1)
                    .is_some_and(|next| next.is_ascii_digit() || next == '(')
                {
                    tokens.push(Token::Star);
                }
            }
            '+' => tokens.push(Token::Plus),
            '-' => tokens.push(Token::Minus),
            '*' => tokens.push(Token::Star),
            '/' => tokens.push(Token::Slash),
            '^' => tokens.push(Token::Caret),
            '%' => tokens.push(Token::Percent),
            '(' => tokens.push(Token::Open),
            ')' => {
                tokens.push(Token::Close);
                if peek(&characters, at + 1)
                    .is_some_and(|next| next.is_ascii_digit() || next == 'x' || next == '(')
                {
                    tokens.push(Token::Star);
                }
            }
            _ => return None,
        }
        at += 1;
    }
    Some(tokens)
}

/// The next non-space character at or after `from`.
fn peek(characters: &[char], from: usize) -> Option<char> {
    characters
        .get(from..)?
        .iter()
        .copied()
        .find(|character| !character.is_whitespace())
}

/// A recursive-descent evaluator over the tokens.
///
/// `x` is the value substituted for [`Token::X`]; the expression itself does not need to
/// contain one.
fn eval(tokens: &[Token], x: Rational) -> Option<Rational> {
    let mut parser = Parser { tokens, at: 0 };
    let value = parser.expression(x)?;
    if parser.at == tokens.len() {
        Some(value)
    } else {
        None
    }
}

struct Parser<'a> {
    tokens: &'a [Token],
    at: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<Token> {
        self.tokens.get(self.at).copied()
    }

    fn expression(&mut self, x: Rational) -> Option<Rational> {
        let mut value = self.term(x)?;
        loop {
            match self.peek() {
                Some(Token::Plus) => {
                    self.at += 1;
                    value = value.add(self.term(x)?)?;
                }
                Some(Token::Minus) => {
                    self.at += 1;
                    value = value.sub(self.term(x)?)?;
                }
                _ => return Some(value),
            }
        }
    }

    fn term(&mut self, x: Rational) -> Option<Rational> {
        let mut value = self.power(x)?;
        loop {
            match self.peek() {
                Some(Token::Star) => {
                    self.at += 1;
                    value = value.mul(self.power(x)?)?;
                }
                Some(Token::Slash) => {
                    self.at += 1;
                    value = value.div(self.power(x)?)?;
                }
                _ => return Some(value),
            }
        }
    }

    fn power(&mut self, x: Rational) -> Option<Rational> {
        let value = self.unary(x)?;
        if self.peek() == Some(Token::Caret) {
            self.at += 1;
            let exponent = self.power(x)?;
            if exponent.den != 1 {
                return None;
            }
            return value.pow(exponent.num);
        }
        Some(value)
    }

    fn unary(&mut self, x: Rational) -> Option<Rational> {
        match self.peek() {
            Some(Token::Plus) => {
                self.at += 1;
                self.unary(x)
            }
            Some(Token::Minus) => {
                self.at += 1;
                // One parse, not two: calling `unary` again would advance the cursor a
                // second time and silently drop a token from the middle of the sum.
                let value = self.unary(x)?;
                Rational::new(value.num.checked_neg()?, value.den)
            }
            _ => self.postfix(x),
        }
    }

    fn postfix(&mut self, x: Rational) -> Option<Rational> {
        let mut value = self.primary(x)?;
        while self.peek() == Some(Token::Percent) {
            self.at += 1;
            value = value.div(Rational::new(100, 1)?)?;
        }
        Some(value)
    }

    fn primary(&mut self, x: Rational) -> Option<Rational> {
        match self.peek()? {
            Token::Number(value) => {
                self.at += 1;
                Some(value)
            }
            Token::X => {
                self.at += 1;
                Some(x)
            }
            Token::Open => {
                self.at += 1;
                let value = self.expression(x)?;
                if self.peek() != Some(Token::Close) {
                    return None;
                }
                self.at += 1;
                Some(value)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The examples the feature exists for, answered exactly.
    #[test]
    fn sums_are_answered_locally() {
        assert_eq!(solve("14 - 8 =").as_deref(), Some("6"));
        assert_eq!(solve("7 multiplied by 87").as_deref(), Some("609"));
        assert_eq!(solve("1/2 + 1/3 = ?").as_deref(), Some("5/6"));
        assert_eq!(solve("2^3 + 1").as_deref(), Some("9"));
        assert_eq!(solve("1.5 + 2.25").as_deref(), Some("3.75"));
        assert_eq!(solve("10 / 4").as_deref(), Some("2.5"));
    }

    /// A percentage question is a computation even though the gate calls it a question.
    #[test]
    fn percentages_are_computed() {
        assert_eq!(solve("What is 15% of 240?").as_deref(), Some("36"));
        assert_eq!(solve("20 percent of 50").as_deref(), Some("10"));
    }

    /// A one-variable linear equation is solved, and the answer names the variable so the
    /// card cannot be mistaken for the value of the expression.
    #[test]
    fn linear_equations_are_solved() {
        assert_eq!(solve("3x + 7 = 22").as_deref(), Some("x = 5"));
        assert_eq!(solve("2x = 10").as_deref(), Some("x = 5"));
        assert_eq!(solve("x + 3 = 7").as_deref(), Some("x = 4"));
        assert_eq!(solve("x/2 = 5").as_deref(), Some("x = 10"));
        assert_eq!(solve("-x + 3 = 1").as_deref(), Some("x = 2"));
    }

    /// **The false positives that matter.** A counter is not a division, an assignment is
    /// not an equation, and prose is not a sum. Each of these was a candidate the gate
    /// could otherwise have sent on.
    #[test]
    fn things_that_only_look_like_math_are_refused() {
        assert_eq!(solve("SCORE 0 / 4"), None);
        assert_eq!(solve("s = \"abc\""), None);
        assert_eq!(solve("The report is due on Friday"), None);
        assert_eq!(solve("x^2 = 4"), None, "a quadratic is not linear");
        assert_eq!(solve("14 - 8 = 6"), None, "the answer is already there");
        assert_eq!(solve("Chapter 3"), None);
        assert_eq!(solve("3 apples and 4 oranges"), None);
    }

    /// A value that cannot be written finitely is a fraction rather than a rounded decimal.
    #[test]
    fn repeating_decimals_stay_exact() {
        assert_eq!(solve("1 / 3").as_deref(), Some("1/3"));
        assert_eq!(solve("2 / 3 + 1 / 3").as_deref(), Some("1"));
    }

    /// The leading words a person uses are stripped; a prefix that is only the start of a
    /// longer word is not.
    #[test]
    fn leading_phrases_are_stripped_but_words_are_not() {
        assert_eq!(solve("Calculate 6 * 7").as_deref(), Some("42"));
        assert_eq!(solve("What is 6 * 7?").as_deref(), Some("42"));
        assert_eq!(solve("whats 6 * 7").as_deref(), Some("42"));
        assert_eq!(solve("solvex 6 * 7"), None);
    }

    /// An expression that overflows or divides by zero declines rather than guessing.
    #[test]
    fn impossible_arithmetic_declines() {
        assert_eq!(solve("1 / 0"), None);
        // Over the exponent bound, and over what an i128 can hold on the way out.
        assert_eq!(solve("2 ^ 999"), None);
        assert_eq!(solve("99999999999999999999 * 99999999999999999999"), None);
    }
}
