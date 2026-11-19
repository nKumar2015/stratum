//! The launcher's calculator: `+ - * / % ^`, parentheses, unary minus and
//! decimals, with the usual precedence (`^` binds tightest, to the right).

/// The value of `input` if it is an arithmetic expression with at least one
/// operator; a bare number is not a calculation.
pub fn evaluate(input: &str) -> Option<f64> {
    let tokens = tokenize(input)?;
    if !tokens.iter().any(|t| matches!(t, Token::Op(_))) {
        return None;
    }
    let mut parser = Parser { tokens: &tokens, at: 0 };
    let value = parser.expression(0)?;
    (parser.at == tokens.len() && value.is_finite()).then_some(value)
}

/// `value` as the launcher shows it: integers without a fraction, others to
/// at most ten significant decimals, trailing zeros dropped.
pub fn format(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        return format!("{value:.0}");
    }
    let text = format!("{value:.10}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Token {
    Number(f64),
    Op(char),
    Open,
    Close,
}

fn tokenize(input: &str) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' | '=' => {}
            '0'..='9' | '.' => {
                let start = i;
                while i + 1 < chars.len() && (chars[i + 1].is_ascii_digit() || chars[i + 1] == '.') {
                    i += 1;
                }
                let text: String = chars[start..=i].iter().collect();
                tokens.push(Token::Number(text.parse().ok()?));
            }
            '+' | '-' | '*' | '/' | '%' | '^' => tokens.push(Token::Op(c)),
            'x' | '×' => tokens.push(Token::Op('*')),
            '÷' => tokens.push(Token::Op('/')),
            '(' => tokens.push(Token::Open),
            ')' => tokens.push(Token::Close),
            _ => return None,
        }
        i += 1;
    }
    (!tokens.is_empty()).then_some(tokens)
}

struct Parser<'a> {
    tokens: &'a [Token],
    at: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<Token> {
        self.tokens.get(self.at).copied()
    }

    /// Precedence climbing over binary operators at or above `min`.
    fn expression(&mut self, min: u8) -> Option<f64> {
        let mut left = self.unary()?;
        while let Some(Token::Op(op)) = self.peek() {
            let (precedence, right_assoc) = match op {
                '+' | '-' => (1, false),
                '*' | '/' | '%' => (2, false),
                '^' => (3, true),
                _ => return None,
            };
            if precedence < min {
                break;
            }
            self.at += 1;
            let right = self.expression(if right_assoc { precedence } else { precedence + 1 })?;
            left = match op {
                '+' => left + right,
                '-' => left - right,
                '*' => left * right,
                '/' => left / right,
                '%' => left % right,
                _ => left.powf(right),
            };
        }
        Some(left)
    }

    fn unary(&mut self) -> Option<f64> {
        match self.peek()? {
            Token::Op('-') => {
                self.at += 1;
                // Binds looser than ^: -2^2 is -4.
                Some(-self.expression(3)?)
            }
            Token::Op('+') => {
                self.at += 1;
                self.unary()
            }
            Token::Number(n) => {
                self.at += 1;
                Some(n)
            }
            Token::Open => {
                self.at += 1;
                let value = self.expression(0)?;
                (self.peek()? == Token::Close).then(|| self.at += 1)?;
                Some(value)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_and_associativity() {
        assert_eq!(evaluate("1 + 2 * 3"), Some(7.0));
        assert_eq!(evaluate("(1 + 2) * 3"), Some(9.0));
        assert_eq!(evaluate("2 ^ 3 ^ 2"), Some(512.0));
        assert_eq!(evaluate("-2^2"), Some(-4.0));
        assert_eq!(evaluate("10 - 4 - 3"), Some(3.0));
        assert_eq!(evaluate("7 % 4"), Some(3.0));
        assert_eq!(evaluate("3x4"), Some(12.0));
        assert_eq!(evaluate("= 1.5 * 2"), Some(3.0));
    }

    #[test]
    fn non_calculations_are_none() {
        assert_eq!(evaluate("42"), None, "a bare number");
        assert_eq!(evaluate("firefox"), None);
        assert_eq!(evaluate("1 +"), None);
        assert_eq!(evaluate("(1 + 2"), None);
        assert_eq!(evaluate("1 / 0"), None, "not finite");
        assert_eq!(evaluate("1..2 + 1"), None);
    }

    #[test]
    fn formatting() {
        assert_eq!(format(12.0), "12");
        assert_eq!(format(0.1 + 0.2), "0.3");
        assert_eq!(format(1.0 / 3.0), "0.3333333333");
        assert_eq!(format(-2.5), "-2.5");
    }
}
