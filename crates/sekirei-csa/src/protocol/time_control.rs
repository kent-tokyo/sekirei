//! CSA time-control parsing.
//!
//! Keep unit conversion and duplicate-field rejection independent from the
//! network session so the tournament clock contract can be tested directly.

use sekirei_core::color::Color;
use std::io;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ParsedTimeControl {
    pub(super) unit_ms: u64,
    pub(super) total_time_ms: Option<u64>,
    pub(super) byoyomi_ms: Option<u64>,
    pub(super) increment_ms: Option<u64>,
}

#[derive(Clone, Debug)]
pub(super) struct CsaTimeControl {
    unit_ms: u64,
    unit_seen: bool,
    total_time: Option<u64>,
    byoyomi: Option<u64>,
    increment: Option<u64>,
}

impl Default for CsaTimeControl {
    fn default() -> Self {
        Self {
            // CSA 1.2.1 defaults to one second when Time_Unit is omitted.
            unit_ms: 1_000,
            unit_seen: false,
            total_time: None,
            byoyomi: None,
            increment: None,
        }
    }
}

impl CsaTimeControl {
    pub(super) fn apply(&mut self, line: &str) -> io::Result<bool> {
        if let Some(value) = line.strip_prefix("Time_Unit:") {
            if self.unit_seen {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "duplicate CSA Time_Unit field",
                ));
            }
            self.unit_ms = parse_time_unit_ms(value)?;
            self.unit_seen = true;
        } else if let Some(value) = line.strip_prefix("Total_Time:") {
            set_once(&mut self.total_time, parse_decimal(value)?, "Total_Time")?;
        } else if let Some(value) = line.strip_prefix("Byoyomi:") {
            set_once(&mut self.byoyomi, parse_decimal(value)?, "Byoyomi")?;
        } else if let Some(value) = line.strip_prefix("Increment:") {
            set_once(&mut self.increment, parse_decimal(value)?, "Increment")?;
        } else {
            return Ok(false);
        }
        Ok(true)
    }

    pub(super) fn finish(self) -> io::Result<ParsedTimeControl> {
        let convert = |name: &str, value: Option<u64>| -> io::Result<Option<u64>> {
            value
                .map(|value| {
                    value.checked_mul(self.unit_ms).ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("{name} overflows milliseconds"),
                        )
                    })
                })
                .transpose()
        };
        Ok(ParsedTimeControl {
            unit_ms: self.unit_ms,
            total_time_ms: convert("Total_Time", self.total_time)?,
            byoyomi_ms: convert("Byoyomi", self.byoyomi)?,
            increment_ms: convert("Increment", self.increment)?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TimeScope {
    Common,
    Black,
    White,
}

/// CSA allows one shared Time section or separate Time+ and Time- sections.
/// Older servers also send the time fields directly in Game_Summary.
#[derive(Clone, Debug, Default)]
pub(super) struct CsaTimeControlSet {
    common: CsaTimeControl,
    black: CsaTimeControl,
    white: CsaTimeControl,
    scope: Option<TimeScope>,
    shared_section: bool,
    black_section: bool,
    white_section: bool,
    common_field: bool,
}

impl CsaTimeControlSet {
    pub(super) fn is_time_line(line: &str) -> bool {
        matches!(
            line,
            "BEGIN Time" | "END Time" | "BEGIN Time+" | "END Time+" | "BEGIN Time-" | "END Time-"
        ) || ["Time_Unit:", "Total_Time:", "Byoyomi:", "Increment:"]
            .iter()
            .any(|prefix| line.starts_with(prefix))
    }

    pub(super) fn apply(&mut self, line: &str) -> io::Result<()> {
        let begin = match line {
            "BEGIN Time" => Some(TimeScope::Common),
            "BEGIN Time+" => Some(TimeScope::Black),
            "BEGIN Time-" => Some(TimeScope::White),
            _ => None,
        };
        if let Some(scope) = begin {
            if self.scope.is_some() {
                return Err(invalid_time("nested CSA Time section"));
            }
            let seen = match scope {
                TimeScope::Common => &mut self.shared_section,
                TimeScope::Black => &mut self.black_section,
                TimeScope::White => &mut self.white_section,
            };
            if *seen {
                return Err(invalid_time("duplicate CSA Time section"));
            }
            *seen = true;
            self.scope = Some(scope);
            return Ok(());
        }
        let end = match line {
            "END Time" => Some(TimeScope::Common),
            "END Time+" => Some(TimeScope::Black),
            "END Time-" => Some(TimeScope::White),
            _ => None,
        };
        if let Some(scope) = end {
            if self.scope != Some(scope) {
                return Err(invalid_time("unmatched CSA Time section end"));
            }
            self.scope = None;
            return Ok(());
        }
        let control = match self.scope {
            Some(TimeScope::Black) => &mut self.black,
            Some(TimeScope::White) => &mut self.white,
            Some(TimeScope::Common) | None => {
                if self.scope.is_none() {
                    self.common_field = true;
                }
                &mut self.common
            }
        };
        control.apply(line)?;
        Ok(())
    }

    pub(super) fn finish(self, color: Color) -> io::Result<ParsedTimeControl> {
        if self.scope.is_some() {
            return Err(invalid_time("unterminated CSA Time section"));
        }
        if self.black_section || self.white_section {
            if !self.black_section
                || !self.white_section
                || self.shared_section
                || self.common_field
            {
                return Err(invalid_time(
                    "CSA Time+ and Time- must be complete and exclusive",
                ));
            }
            return if color == Color::Black {
                self.black.finish()
            } else {
                self.white.finish()
            };
        }
        self.common.finish()
    }
}

fn invalid_time(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub(super) fn parse_decimal(value: &str) -> io::Result<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "CSA time value must contain decimal digits only",
        ));
    }
    value.parse().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "CSA time value is outside the supported range",
        )
    })
}

pub(super) fn parse_time_unit_ms(value: &str) -> io::Result<u64> {
    let (digits, multiplier) = if let Some(digits) = value.strip_suffix("msec") {
        (digits, 1)
    } else if let Some(digits) = value.strip_suffix("sec") {
        (digits, 1_000)
    } else if let Some(digits) = value.strip_suffix("min") {
        (digits, 60_000)
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported CSA Time_Unit",
        ));
    };
    let amount = parse_decimal(digits)?;
    if amount == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "CSA Time_Unit must be positive",
        ));
    }
    amount.checked_mul(multiplier).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "CSA Time_Unit overflows milliseconds",
        )
    })
}

fn set_once(slot: &mut Option<u64>, value: u64, name: &str) -> io::Result<()> {
    if slot.replace(value).is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("duplicate CSA {name} field"),
        ));
    }
    Ok(())
}
