//! Parser for the console's deliberately small textual surface.

use std::iter::Peekable;
use std::str::Chars;

use crate::{
    BinaryOperator, ConsoleError, ConsolePlan, ConsoleResult, ConsoleScalar, PipelinePlan,
    PipelineTerminal, RowSetPlan, ScalarExpression, Selection, WritePlan,
};

/// A parsed read or mutation program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsoleProgram {
    /// A schema, point read, view, scan, or aggregate.
    Read(ConsolePlan),
    /// A point or bounded same-table mutation that is staged when executed.
    Write(WritePlan),
}

/// Parses one textual program into the same typed plans used by Rust callers.
///
/// `scan_limit` bounds storage reads before filters and `take` are applied. Text programs cannot
/// override or remove this outer bound.
pub fn parse_console_program(program: &str, scan_limit: usize) -> ConsoleResult<ConsoleProgram> {
    Parser::new(program)?.parse(scan_limit)
}

/// Parses a read-only program, rejecting setter and modifier pipelines.
pub fn parse_console_plan(program: &str, scan_limit: usize) -> ConsoleResult<ConsolePlan> {
    match parse_console_program(program, scan_limit)? {
        ConsoleProgram::Read(plan) => Ok(plan),
        ConsoleProgram::Write(_) => Err(parse_error("write program is not allowed here")),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Word(String),
    String(String),
    Number(String),
    Bytes(Vec<u8>),
    LeftParen,
    RightParen,
    Comma,
    Pipe,
    And,
    Or,
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Plus,
    Minus,
    Multiply,
    Divide,
    Not,
    End,
}

struct Parser {
    tokens: Vec<Token>,
    cursor: usize,
}

impl Parser {
    fn new(program: &str) -> ConsoleResult<Self> {
        Ok(Self {
            tokens: tokenize(program)?,
            cursor: 0,
        })
    }

    fn parse(mut self, scan_limit: usize) -> ConsoleResult<ConsoleProgram> {
        let command = self.take_word("command")?;
        match command.as_str() {
            "schema" => {
                let source = self.take_word("schema source")?;
                self.expect_end()?;
                Ok(ConsoleProgram::Read(ConsolePlan::schema(source)))
            }
            "get" => {
                let source = self.take_word("get source")?;
                let mut arguments = Vec::new();
                while !matches!(self.peek(), Token::Pipe | Token::End) {
                    arguments.push(self.parse_argument()?);
                }
                if arguments.is_empty() {
                    return Err(parse_error(
                        "get requires at least one key or view argument",
                    ));
                }
                if self.peek() == &Token::End {
                    Ok(ConsoleProgram::Read(ConsolePlan::get(source, arguments)))
                } else {
                    self.parse_point_write(source, arguments)
                }
            }
            "scan" => self.parse_pipeline(false, scan_limit),
            "scan_rev" => self.parse_pipeline(true, scan_limit),
            _ => Err(parse_error(format!("unknown command '{command}'"))),
        }
    }

    fn parse_pipeline(
        &mut self,
        reverse: bool,
        scan_limit: usize,
    ) -> ConsoleResult<ConsoleProgram> {
        let table = self.take_word("scan table")?;
        let mut rows = if reverse {
            RowSetPlan::scan_rev(table, scan_limit)?
        } else {
            RowSetPlan::scan(table, scan_limit)?
        };
        let mut seen = [false; 3];
        let mut selections = None;

        while self.peek() != &Token::End {
            self.expect(Token::Pipe, "expected '|' before pipeline operation")?;
            let operation = self.take_word("pipeline operation")?;
            match operation.as_str() {
                "filter" => {
                    mark_once(&mut seen[0], "filter")?;
                    if seen[1] || seen[2] {
                        return Err(parse_error("filter must precede select and take"));
                    }
                    let expression = self.parse_expression(0)?;
                    self.expect_stage_end()?;
                    rows = rows.filter(expression);
                }
                "select" => {
                    mark_once(&mut seen[1], "select")?;
                    if seen[2] {
                        return Err(parse_error("select must precede take"));
                    }
                    selections = Some(self.parse_selections()?);
                }
                "take" => {
                    mark_once(&mut seen[2], "take")?;
                    let limit = self.take_usize("take limit")?;
                    self.expect_stage_end()?;
                    rows = rows.take(limit)?;
                }
                "first" | "last" | "count" => {
                    self.expect_stage_end()?;
                    let terminal = match operation.as_str() {
                        "first" => PipelineTerminal::First,
                        "last" => PipelineTerminal::Last,
                        "count" => PipelineTerminal::Count,
                        _ => unreachable!("matched fixed terminal name"),
                    };
                    self.expect_end()?;
                    return Ok(ConsoleProgram::Read(finish_pipeline(
                        rows, selections, terminal,
                    )));
                }
                "sum" | "min" | "max" | "any" | "all" => {
                    let expression = self.parse_expression(0)?;
                    self.expect_stage_end()?;
                    let terminal = match operation.as_str() {
                        "sum" => PipelineTerminal::Sum(expression),
                        "min" => PipelineTerminal::Min(expression),
                        "max" => PipelineTerminal::Max(expression),
                        "any" => PipelineTerminal::Any(expression),
                        "all" => PipelineTerminal::All(expression),
                        _ => unreachable!("matched fixed aggregate name"),
                    };
                    self.expect_end()?;
                    return Ok(ConsoleProgram::Read(finish_pipeline(
                        rows, selections, terminal,
                    )));
                }
                "modify" => {
                    if selections.is_some() {
                        return Err(parse_error("modify cannot follow select"));
                    }
                    let modifier = self.take_word("modifier name")?;
                    let arguments = self.take_remaining_arguments()?;
                    return Ok(ConsoleProgram::Write(WritePlan::ModifyRows {
                        rows,
                        modifier,
                        arguments,
                    }));
                }
                _ => {
                    return Err(parse_error(format!(
                        "unknown pipeline operation '{operation}'"
                    )));
                }
            }
        }
        Ok(ConsoleProgram::Read(finish_pipeline(
            rows,
            selections,
            PipelineTerminal::Rows,
        )))
    }

    fn parse_point_write(
        &mut self,
        table: String,
        mut arguments: Vec<ConsoleScalar>,
    ) -> ConsoleResult<ConsoleProgram> {
        if arguments.len() != 1 {
            return Err(parse_error("point writes require exactly one table key"));
        }
        let key = arguments.pop().expect("one point key was checked");
        self.expect(Token::Pipe, "expected '|' before write operation")?;
        let operation = self.take_word("write operation")?;
        let plan = match operation.as_str() {
            "set" => WritePlan::Set {
                table,
                key,
                field: self.take_word("field name")?,
                value: self.parse_argument()?,
            },
            "modify" => WritePlan::Modify {
                table,
                key,
                modifier: self.take_word("modifier name")?,
                arguments: self.take_remaining_arguments()?,
            },
            _ => {
                return Err(parse_error(format!(
                    "unknown write operation '{operation}'"
                )));
            }
        };
        self.expect_end()?;
        Ok(ConsoleProgram::Write(plan))
    }

    fn take_remaining_arguments(&mut self) -> ConsoleResult<Vec<ConsoleScalar>> {
        let mut arguments = Vec::new();
        while self.peek() != &Token::End {
            arguments.push(self.parse_argument()?);
        }
        Ok(arguments)
    }

    fn parse_selections(&mut self) -> ConsoleResult<Vec<Selection>> {
        let mut selections = Vec::new();
        loop {
            let expression = self.parse_expression(0)?;
            let name = if self.peek_word("as") {
                self.bump();
                self.take_word("selection name")?
            } else {
                match &expression {
                    ScalarExpression::Key => "key".to_owned(),
                    ScalarExpression::Field(name) => name.clone(),
                    _ => {
                        return Err(parse_error(
                            "computed selections require an explicit 'as <name>' alias",
                        ));
                    }
                }
            };
            selections.push(Selection::new(name, expression));
            match self.peek() {
                Token::Comma => self.bump(),
                Token::Pipe | Token::End => return Ok(selections),
                _ => return Err(parse_error("expected ',' or the end of select")),
            }
        }
    }

    fn parse_expression(&mut self, minimum_precedence: u8) -> ConsoleResult<ScalarExpression> {
        let mut left = self.parse_primary()?;
        while let Some((operator, precedence)) = binary_operator(self.peek()) {
            if precedence < minimum_precedence {
                break;
            }
            self.bump();
            let right = self.parse_expression(precedence + 1)?;
            left = ScalarExpression::binary(operator, left, right);
        }
        Ok(left)
    }

    fn parse_primary(&mut self) -> ConsoleResult<ScalarExpression> {
        match self.peek().clone() {
            Token::Not => {
                self.bump();
                Ok(ScalarExpression::Not(Box::new(self.parse_primary()?)))
            }
            Token::Minus => {
                self.bump();
                self.take_i64()
                    .map(ConsoleScalar::I64)
                    .map(ScalarExpression::literal)
            }
            Token::LeftParen => {
                self.bump();
                let expression = self.parse_expression(0)?;
                self.expect(Token::RightParen, "expected ')' after expression")?;
                Ok(expression)
            }
            Token::String(value) => {
                self.bump();
                Ok(ScalarExpression::literal(ConsoleScalar::String(value)))
            }
            Token::Number(number) => {
                self.bump();
                parse_u64(&number)
                    .map(ConsoleScalar::U64)
                    .map(ScalarExpression::literal)
            }
            Token::Bytes(bytes) => {
                self.bump();
                Ok(ScalarExpression::literal(ConsoleScalar::Bytes(bytes)))
            }
            Token::Word(word) => {
                self.bump();
                Ok(match word.as_str() {
                    "key" => ScalarExpression::Key,
                    "null" => ScalarExpression::literal(ConsoleScalar::Null),
                    "true" => ScalarExpression::literal(ConsoleScalar::Bool(true)),
                    "false" => ScalarExpression::literal(ConsoleScalar::Bool(false)),
                    _ => ScalarExpression::field(word),
                })
            }
            _ => Err(parse_error("expected an expression")),
        }
    }

    fn parse_argument(&mut self) -> ConsoleResult<ConsoleScalar> {
        match self.peek().clone() {
            Token::String(value) => {
                self.bump();
                Ok(ConsoleScalar::String(value))
            }
            Token::Number(number) => {
                self.bump();
                parse_u64(&number).map(ConsoleScalar::U64)
            }
            Token::Minus => {
                self.bump();
                self.take_i64().map(ConsoleScalar::I64)
            }
            Token::Bytes(bytes) => {
                self.bump();
                Ok(ConsoleScalar::Bytes(bytes))
            }
            Token::Word(word) => {
                self.bump();
                match word.as_str() {
                    "null" => Ok(ConsoleScalar::Null),
                    "true" => Ok(ConsoleScalar::Bool(true)),
                    "false" => Ok(ConsoleScalar::Bool(false)),
                    _ if is_bare_hex(&word) => decode_hex(&word).map(ConsoleScalar::Bytes),
                    _ => Ok(ConsoleScalar::String(word)),
                }
            }
            _ => Err(parse_error("expected a scalar argument")),
        }
    }

    fn take_i64(&mut self) -> ConsoleResult<i64> {
        let number = self.take_number("signed integer")?;
        format!("-{number}")
            .parse()
            .map_err(|_| parse_error("signed integer is out of range"))
    }

    fn take_usize(&mut self, target: &'static str) -> ConsoleResult<usize> {
        self.take_number(target)?
            .parse()
            .map_err(|_| parse_error(format!("{target} is out of range")))
    }

    fn take_number(&mut self, target: &'static str) -> ConsoleResult<String> {
        match self.peek().clone() {
            Token::Number(number) => {
                self.bump();
                Ok(number)
            }
            _ => Err(parse_error(format!("expected {target}"))),
        }
    }

    fn take_word(&mut self, target: &'static str) -> ConsoleResult<String> {
        match self.peek().clone() {
            Token::Word(word) => {
                self.bump();
                Ok(word)
            }
            _ => Err(parse_error(format!("expected {target}"))),
        }
    }

    fn expect(&mut self, token: Token, message: &'static str) -> ConsoleResult<()> {
        if self.peek() == &token {
            self.bump();
            Ok(())
        } else {
            Err(parse_error(message))
        }
    }

    fn expect_stage_end(&self) -> ConsoleResult<()> {
        if matches!(self.peek(), Token::Pipe | Token::End) {
            Ok(())
        } else {
            Err(parse_error("unexpected token after pipeline operation"))
        }
    }

    fn expect_end(&self) -> ConsoleResult<()> {
        if self.peek() == &Token::End {
            Ok(())
        } else {
            Err(parse_error("unexpected trailing input"))
        }
    }

    fn peek_word(&self, expected: &str) -> bool {
        matches!(self.peek(), Token::Word(word) if word == expected)
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.cursor]
    }

    fn bump(&mut self) {
        if self.peek() != &Token::End {
            self.cursor += 1;
        }
    }
}

fn finish_pipeline(
    rows: RowSetPlan,
    selections: Option<Vec<Selection>>,
    terminal: PipelineTerminal,
) -> ConsolePlan {
    let mut plan = PipelinePlan::from(rows);
    if let Some(selections) = selections {
        plan = plan.select(selections);
    }
    plan.terminal(terminal).into()
}

fn mark_once(seen: &mut bool, operation: &'static str) -> ConsoleResult<()> {
    if *seen {
        Err(parse_error(format!("{operation} may appear only once")))
    } else {
        *seen = true;
        Ok(())
    }
}

fn binary_operator(token: &Token) -> Option<(BinaryOperator, u8)> {
    Some(match token {
        Token::Or => (BinaryOperator::Or, 1),
        Token::And => (BinaryOperator::And, 2),
        Token::Equal => (BinaryOperator::Equal, 3),
        Token::NotEqual => (BinaryOperator::NotEqual, 3),
        Token::Less => (BinaryOperator::Less, 4),
        Token::LessOrEqual => (BinaryOperator::LessOrEqual, 4),
        Token::Greater => (BinaryOperator::Greater, 4),
        Token::GreaterOrEqual => (BinaryOperator::GreaterOrEqual, 4),
        Token::Plus => (BinaryOperator::Add, 5),
        Token::Minus => (BinaryOperator::Subtract, 5),
        Token::Multiply => (BinaryOperator::Multiply, 6),
        Token::Divide => (BinaryOperator::Divide, 6),
        _ => return None,
    })
}

fn tokenize(program: &str) -> ConsoleResult<Vec<Token>> {
    let mut characters = program.chars().peekable();
    let mut tokens = Vec::new();
    while let Some(character) = characters.next() {
        if character.is_whitespace() {
            continue;
        }
        let token = match character {
            '(' => Token::LeftParen,
            ')' => Token::RightParen,
            ',' => Token::Comma,
            '+' => Token::Plus,
            '-' => Token::Minus,
            '*' => Token::Multiply,
            '/' => Token::Divide,
            '|' => paired_or_single(&mut characters, '|', Token::Or, Token::Pipe),
            '&' => paired(&mut characters, '&', Token::And)?,
            '=' => paired(&mut characters, '=', Token::Equal)?,
            '!' => paired_or_single(&mut characters, '=', Token::NotEqual, Token::Not),
            '<' => paired_or_single(&mut characters, '=', Token::LessOrEqual, Token::Less),
            '>' => paired_or_single(&mut characters, '=', Token::GreaterOrEqual, Token::Greater),
            '"' => Token::String(read_string(&mut characters)?),
            character if character.is_ascii_digit() => read_number(character, &mut characters)?,
            character if is_word_start(character) => {
                Token::Word(read_while(character, &mut characters, is_word_continue))
            }
            _ => return Err(parse_error(format!("unexpected character '{character}'"))),
        };
        tokens.push(token);
    }
    tokens.push(Token::End);
    Ok(tokens)
}

fn paired_or_single(
    characters: &mut Peekable<Chars<'_>>,
    second: char,
    paired: Token,
    single: Token,
) -> Token {
    if characters.next_if_eq(&second).is_some() {
        paired
    } else {
        single
    }
}

fn paired(
    characters: &mut Peekable<Chars<'_>>,
    second: char,
    token: Token,
) -> ConsoleResult<Token> {
    characters
        .next_if_eq(&second)
        .map(|_| token)
        .ok_or_else(|| parse_error(format!("expected '{second}' twice")))
}

fn read_number(first: char, characters: &mut Peekable<Chars<'_>>) -> ConsoleResult<Token> {
    if first == '0'
        && characters
            .next_if(|character| matches!(character, 'x' | 'X'))
            .is_some()
    {
        let hex = read_while_optional(characters, char::is_ascii_hexdigit);
        return decode_hex(&hex).map(Token::Bytes);
    }
    let value = read_while(first, characters, char::is_ascii_alphanumeric);
    if value.chars().all(|character| character.is_ascii_digit()) {
        Ok(Token::Number(value))
    } else {
        decode_hex(&value).map(Token::Bytes)
    }
}

fn read_string(characters: &mut Peekable<Chars<'_>>) -> ConsoleResult<String> {
    let mut value = String::new();
    while let Some(character) = characters.next() {
        match character {
            '"' => return Ok(value),
            '\\' => value.push(match characters.next() {
                Some('"') => '"',
                Some('\\') => '\\',
                Some('n') => '\n',
                Some('r') => '\r',
                Some('t') => '\t',
                _ => return Err(parse_error("unsupported or unterminated string escape")),
            }),
            _ => value.push(character),
        }
    }
    Err(parse_error("unterminated string literal"))
}

fn read_while(
    first: char,
    characters: &mut Peekable<Chars<'_>>,
    predicate: fn(&char) -> bool,
) -> String {
    let mut value = first.to_string();
    value.push_str(&read_while_optional(characters, predicate));
    value
}

fn read_while_optional(
    characters: &mut Peekable<Chars<'_>>,
    predicate: fn(&char) -> bool,
) -> String {
    let mut value = String::new();
    while let Some(character) = characters.next_if(predicate) {
        value.push(character);
    }
    value
}

fn is_word_start(character: char) -> bool {
    character.is_ascii_alphabetic() || character == '_'
}

fn is_word_continue(character: &char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '.')
}

fn is_bare_hex(value: &str) -> bool {
    !value.is_empty()
        && value.len().is_multiple_of(2)
        && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn decode_hex(value: &str) -> ConsoleResult<Vec<u8>> {
    if value.is_empty() || !value.len().is_multiple_of(2) {
        return Err(parse_error(
            "hex bytes require a non-empty even number of digits",
        ));
    }
    hex::decode(value).map_err(|_| parse_error("invalid hex byte string"))
}

fn parse_u64(value: &str) -> ConsoleResult<u64> {
    value
        .parse()
        .map_err(|_| parse_error("unsigned integer is out of range"))
}

fn parse_error(message: impl Into<String>) -> ConsoleError {
    ConsoleError::invalid_input("console program", message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_point_reads_and_fixed_pipelines() {
        assert_eq!(
            parse_console_plan("get tasks 12adbeef", 100).expect("test: parse point read"),
            ConsolePlan::get(
                "tasks",
                vec![ConsoleScalar::Bytes(vec![0x12, 0xad, 0xbe, 0xef])]
            )
        );

        let expected = PipelinePlan::scan_rev("tasks", 100)
            .expect("test: build scan")
            .filter(ScalarExpression::binary(
                BinaryOperator::Equal,
                ScalarExpression::field("status"),
                ScalarExpression::literal(ConsoleScalar::String("pending".to_owned())),
            ))
            .select(vec![
                Selection::new("key", ScalarExpression::Key),
                Selection::new(
                    "retry_plus_four",
                    ScalarExpression::binary(
                        BinaryOperator::Add,
                        ScalarExpression::field("retry_after_secs"),
                        ScalarExpression::literal(ConsoleScalar::U64(4)),
                    ),
                ),
            ])
            .take(20)
            .expect("test: add take limit");
        assert_eq!(
            parse_console_plan(
                "scan_rev tasks | filter status == \"pending\" | \
                 select key, retry_after_secs + 4 as retry_plus_four | take 20",
                100,
            )
            .expect("test: parse pipeline"),
            ConsolePlan::from(expected)
        );
    }

    #[test]
    fn parses_mutations_as_implicit_staged_writes() {
        assert_eq!(
            parse_console_program("get tasks 0x12 | set retry_after_secs null", 100)
                .expect("test: parse point setter"),
            ConsoleProgram::Write(WritePlan::Set {
                table: "tasks".to_owned(),
                key: ConsoleScalar::Bytes(vec![0x12]),
                field: "retry_after_secs".to_owned(),
                value: ConsoleScalar::Null,
            })
        );

        let rows = RowSetPlan::scan("tasks", 100)
            .expect("test: build bounded row selection")
            .filter(ScalarExpression::binary(
                BinaryOperator::Equal,
                ScalarExpression::field("status"),
                ScalarExpression::literal(ConsoleScalar::String("pending".to_owned())),
            ))
            .take(20)
            .expect("test: add take limit");
        assert_eq!(
            parse_console_program(
                "scan tasks | filter status == \"pending\" | take 20 | \
                 modify abandon \"operator cancelled\"",
                100,
            )
            .expect("test: parse bulk modifier"),
            ConsoleProgram::Write(WritePlan::ModifyRows {
                rows,
                modifier: "abandon".to_owned(),
                arguments: vec![ConsoleScalar::String("operator cancelled".to_owned())],
            })
        );

        assert!(
            parse_console_plan("get tasks 0x12 | set retry_after_secs null", 100).is_err(),
            "read-only parsing must reject writes"
        );
        assert!(
            parse_console_program("get tasks 0x12 | set retry_after_secs null | stage", 100)
                .is_err(),
            "stage is implicit and not a pipeline operation"
        );
    }

    #[test]
    fn rejects_open_ended_or_extensible_programs() {
        assert!(parse_console_plan("scan tasks", 0).is_err());
        assert!(parse_console_plan("scan tasks | map status", 100).is_err());
        assert!(parse_console_plan("scan tasks | filter status = \"pending\"", 100).is_err());
    }
}
