//! Field parsing and expansion — port of `croniter._expand` (croniter.py:944)
//! and `croniter.expand` (croniter.py:1240).
//!
//! Turns `"0-5,10 * * * mon-fri"` into per-field sorted value lists, plus the
//! two side-tables croniter carries separately: `nth_weekday_of_month` (the `#`
//! and `lN` syntax) and `nearest_weekday` (the `W` syntax).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::LazyLock;

use chrono::{DateTime, Datelike, Timelike, Utc};
use regex::Regex;

use crate::consts::*;
use crate::error::{CroniterError, Result};
use crate::hash;

/// One expanded value in a field.
///
/// Python stores `int | "*" | "l"` in the same list and sorts with
/// `key=lambda i: f"{i:02}" if isinstance(i, int) else i`, i.e. lexicographic
/// on the formatted string. Since `"*"` is 0x2A, digits are 0x30-0x39 and
/// `"l"` is 0x6C, that ordering is exactly `Star < Num < Last` with `Num`
/// compared numerically (every field's values share a digit width). The
/// derived `Ord` below reproduces it — the variant order is load-bearing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Item {
    Star,
    Num(i64),
    Last,
}

impl Item {
    pub fn as_num(&self) -> Option<i64> {
        match self {
            Item::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn is_star(&self) -> bool {
        matches!(self, Item::Star)
    }
}

impl std::fmt::Display for Item {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Item::Star => write!(f, "*"),
            Item::Last => write!(f, "l"),
            Item::Num(n) => write!(f, "{n}"),
        }
    }
}

/// The `nth` qualifier in `5#3` (third Friday) or `l3` (last Wednesday).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Nth {
    Num(i64),
    Last,
}

/// Result of expanding a full cron expression.
#[derive(Clone, Debug)]
pub struct Expanded {
    pub expanded: Vec<Vec<Item>>,
    pub nth_weekday_of_month: BTreeMap<Item, BTreeSet<Nth>>,
    pub expressions: Vec<String>,
    pub nearest_weekday: BTreeSet<i64>,
}

static STEP_SEARCH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([^-]+)-([^-/]+)(/(\d+))?$").unwrap());
static ONLY_INT_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+$").unwrap());
static STAR_OR_INT_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d+|\*)$").unwrap());
static STAR_SLASH_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\*(/.+)$").unwrap());
static VALUE_SLASH_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(.+)/(.+)$").unwrap());
static NEAREST_WEEKDAY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:(\d+)w|w(\d+))$").unwrap());

static SPECIAL_DOW_RE: LazyLock<Regex> = LazyLock::new(|| {
    let weekdays = DOW_ALPHAS
        .iter()
        .map(|(k, _)| *k)
        .collect::<Vec<_>>()
        .join("|");
    let months = M_ALPHAS
        .iter()
        .map(|(k, _)| *k)
        .collect::<Vec<_>>()
        .join("|");
    Regex::new(&format!(
        r"^(?P<pre>((?P<he>(({weekdays})(-({weekdays}))?)|(({months})(-({months}))?)|\w+)#)|l)(?P<last>\d+)$"
    ))
    .unwrap()
});

/// `@daily` and friends (croniter.py:948). Index 0 is the plain form, index 1
/// the form used when a `hash_id` was supplied.
const EXPR_ALIASES: [(&str, &str, &str); 7] = [
    ("@midnight", "0 0 * * *", "h h(0-2) * * * h"),
    ("@hourly", "0 * * * *", "h * * * * h"),
    ("@daily", "0 0 * * *", "h h * * * h"),
    ("@weekly", "0 0 * * 0", "h h * * h h"),
    ("@monthly", "0 0 1 * *", "h h h * * h"),
    ("@yearly", "0 0 1 1 *", "h h h h * h"),
    ("@annually", "0 0 1 1 *", "h h h h * h"),
];

/// `croniter.value_alias` (croniter.py:923) — the 0->1 / 7->0 remaps, which are
/// suppressed for particular (field, column-count) combinations.
pub fn value_alias(val: Item, field_index: usize, len_expressions: usize) -> Item {
    let mapped = match (field_index, val) {
        (DAY_FIELD, Item::Num(0)) => Some(Item::Num(1)),
        (MONTH_FIELD, Item::Num(0)) => Some(Item::Num(1)),
        (DOW_FIELD, Item::Num(7)) => Some(Item::Num(0)),
        _ => None,
    };
    let Some(mapped) = mapped else { return val };

    let suppressed = ((field_index == DAY_FIELD || field_index == MONTH_FIELD)
        && len_expressions == UNIX_CRON_LEN)
        || ((field_index == MONTH_FIELD || field_index == DOW_FIELD)
            && len_expressions == SECOND_CRON_LEN)
        || ((field_index == DAY_FIELD || field_index == MONTH_FIELD || field_index == DOW_FIELD)
            && len_expressions == YEAR_CRON_LEN);

    if suppressed {
        val
    } else {
        mapped
    }
}

fn alphaconv(index: usize, key: &str, expressions: &[String]) -> Result<Item> {
    let table: &[(&str, i64)] = match index {
        MONTH_FIELD => &M_ALPHAS,
        DOW_FIELD => &DOW_ALPHAS,
        DAY_FIELD if key == "l" => return Ok(Item::Last),
        _ => &[],
    };
    for (name, v) in table {
        if *name == key {
            return Ok(Item::Num(*v));
        }
    }
    Err(CroniterError::NotAlpha(format!(
        "[{}] is not acceptable",
        expressions.join(" ")
    )))
}

/// `croniter._get_low_from_current_date_number` (croniter.py:1304).
fn low_from_current_date_number(field_index: usize, step: i64, from_timestamp: i64) -> Result<i64> {
    let dt: DateTime<Utc> = DateTime::from_timestamp(from_timestamp, 0)
        .ok_or_else(|| CroniterError::Value("timestamp out of range".to_string()))?;
    Ok(match field_index {
        MINUTE_FIELD => i64::from(dt.minute()) % step,
        HOUR_FIELD => i64::from(dt.hour()) % step,
        DAY_FIELD => ((i64::from(dt.day()) - 1) % step) + 1,
        MONTH_FIELD => ((i64::from(dt.month()) - 1) % step) + 1,
        DOW_FIELD => (i64::from(dt.weekday().number_from_monday()) % 7) % step,
        _ => {
            return Err(CroniterError::Value(
                "Can't get current date number for index larger than 4".to_string(),
            ))
        }
    })
}

/// `croniter._expand` (croniter.py:944).
pub fn expand(
    expr_format: &str,
    hash_id: Option<&[u8]>,
    second_at_beginning: bool,
    from_timestamp: Option<f64>,
    strict: bool,
    strict_year: Option<&[i64]>,
) -> Result<Expanded> {
    let efl_lower = expr_format.to_lowercase();
    let hash_id_expr = usize::from(hash_id.is_some());
    let efl = EXPR_ALIASES
        .iter()
        .find(|(alias, _, _)| *alias == efl_lower)
        .map(|(_, plain, hashed)| {
            if hash_id_expr == 1 {
                (*hashed).to_string()
            } else {
                (*plain).to_string()
            }
        })
        .unwrap_or(efl_lower);

    let mut expressions: Vec<String> = efl.split_whitespace().map(str::to_string).collect();

    if !VALID_LEN_EXPRESSION.contains(&expressions.len()) {
        return Err(CroniterError::BadCron(
            "Exactly 5, 6 or 7 columns has to be specified for iterator expression.".to_string(),
        ));
    }

    if expressions.len() > UNIX_CRON_LEN && second_at_beginning {
        // move second to its own (6th) field so the same logic applies
        let first = expressions.remove(0);
        expressions.insert(SECOND_FIELD, first);
    }

    let len_expressions = expressions.len();
    let mut expanded: Vec<Vec<Item>> = Vec::with_capacity(len_expressions);
    let mut nth_weekday_of_month: BTreeMap<Item, BTreeSet<Nth>> = BTreeMap::new();
    let mut nearest_weekday: BTreeSet<i64> = BTreeSet::new();

    for field_index in 0..len_expressions {
        let mut expr = expressions[field_index].clone();

        // EXPANDERS — currently just the hash expander (croniter.py:1531).
        expr = hash::expand_field(field_index, &expr, hash_id)?;

        if expr.contains('?') {
            if expr != "?" {
                return Err(CroniterError::BadCron(format!(
                    "[{expr_format}] is not acceptable. Question mark can not used with other characters"
                )));
            }
            if field_index != DAY_FIELD && field_index != DOW_FIELD {
                return Err(CroniterError::BadCron(format!(
                    "[{expr_format}] is not acceptable. Question mark can only used in day_of_month or day_of_week"
                )));
            }
            // currently just treat `?` as `*`
            expr = "*".to_string();
        }

        let mut e_list: Vec<String> = expr.split(',').map(str::to_string).collect();
        let mut res: Vec<Item> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        while let Some(e) = e_list.pop() {
            let mut e = e;
            let mut nth: Option<Nth> = None;

            if field_index == DOW_FIELD {
                // Special case in the dow expression: 2#3, l3
                let special = SPECIAL_DOW_RE.captures(&e).map(|caps| {
                    (
                        caps.name("he")
                            .map(|m| m.as_str())
                            .unwrap_or("")
                            .to_string(),
                        caps.name("last")
                            .map(|m| m.as_str())
                            .unwrap_or("")
                            .to_string(),
                    )
                });
                if let Some((he, last)) = special {
                    if !he.is_empty() {
                        e = he.to_string();
                        let parsed = last.parse::<i64>().ok().filter(|n| (1..=5).contains(n));
                        match parsed {
                            Some(n) => nth = Some(Nth::Num(n)),
                            None => {
                                return Err(CroniterError::BadCron(format!(
                                    "[{expr_format}] is not acceptable. Invalid day_of_week value: '{last}'"
                                )))
                            }
                        }
                    } else if !last.is_empty() {
                        e = last.to_string();
                        nth = Some(Nth::Last); // g["pre"] == 'l'
                    }
                }
            }

            if field_index == DAY_FIELD {
                // W (nearest weekday) in day-of-month: 15w, w15
                if let Some(caps) = NEAREST_WEEKDAY_RE.captures(&e) {
                    let raw = caps
                        .get(1)
                        .or_else(|| caps.get(2))
                        .map(|m| m.as_str())
                        .unwrap_or("");
                    let w_day: i64 = raw.parse().map_err(|_| {
                        CroniterError::BadCron(format!("[{expr_format}] is not acceptable"))
                    })?;
                    if !(1..=31).contains(&w_day) {
                        return Err(CroniterError::BadCron(format!(
                            "[{expr_format}] is not acceptable, nearest weekday day value '{w_day}' out of range"
                        )));
                    }
                    if !e_list.is_empty() || !res.is_empty() {
                        return Err(CroniterError::BadCron(format!(
                            "[{expr_format}] is not acceptable. 'W' can only be used with a single day value, not in a list or range"
                        )));
                    }
                    nearest_weekday.insert(w_day);
                    res.push(Item::Num(w_day));
                    continue;
                }
            }

            // Normalize "*/5" to "{min}-{max}/5" before matching step_search_re.
            let (lo_r, hi_r) = RANGES[field_index];
            let mut t = if let Some(caps) = STAR_SLASH_RE.captures(&e) {
                format!("{}-{}{}", lo_r, hi_r, &caps[1])
            } else {
                e.clone()
            };
            let mut m = STEP_SEARCH_RE.captures(&t);

            if m.is_none() {
                // Normalize "{start}/{step}" to "{start}-{max}/{step}".
                if let Some(caps) = VALUE_SLASH_RE.captures(&e) {
                    t = format!("{}-{}/{}", &caps[1], hi_r, &caps[2]);
                    m = STEP_SEARCH_RE.captures(&t);
                }
            }

            if let Some(caps) = m {
                let mut low = caps[1].to_string();
                let mut high = caps[2].to_string();
                let step_str = caps.get(4).map(|x| x.as_str()).unwrap_or("1").to_string();

                if field_index == DAY_FIELD && high == "l" {
                    high = "31".to_string();
                }

                if !ONLY_INT_RE.is_match(&low) {
                    low = alphaconv(field_index, &low, &expressions)?.to_string();
                }
                if !ONLY_INT_RE.is_match(&high) {
                    high = alphaconv(field_index, &high, &expressions)?.to_string();
                }

                if !ONLY_INT_RE.is_match(&step_str) {
                    return Err(CroniterError::BadCron(format!(
                        "[{expr_format}] step '{step_str}' in field {field_index} is not acceptable"
                    )));
                }
                let step: i64 = step_str.parse().map_err(|_| {
                    CroniterError::BadCron(format!(
                        "[{expr_format}] step '{step_str}' in field {field_index} is not acceptable"
                    ))
                })?;
                if step == 0 {
                    return Err(CroniterError::BadCron(format!(
                        "[{expr_format}] step '{step}' in field {field_index} is not acceptable"
                    )));
                }

                for band in [&low, &high] {
                    if !ONLY_INT_RE.is_match(band) {
                        return Err(CroniterError::BadCron(format!(
                            "[{expr_format}] bands '{low}-{high}' in field {field_index} are not acceptable"
                        )));
                    }
                }

                let parse_band = |s: &str| -> Result<i64> {
                    s.parse::<i64>().map_err(|_| {
                        CroniterError::BadCron(format!(
                            "[{expr_format}] bands '{low}-{high}' in field {field_index} are not acceptable"
                        ))
                    })
                };
                let mut low_v =
                    match value_alias(Item::Num(parse_band(&low)?), field_index, len_expressions) {
                        Item::Num(n) => n,
                        other => {
                            return Err(CroniterError::BadCron(format!("unexpected band {other}")))
                        }
                    };
                let high_v = match value_alias(
                    Item::Num(parse_band(&high)?),
                    field_index,
                    len_expressions,
                ) {
                    Item::Num(n) => n,
                    other => {
                        return Err(CroniterError::BadCron(format!("unexpected band {other}")))
                    }
                };

                if low_v.max(high_v) > lo_r.max(hi_r) {
                    return Err(CroniterError::BadCron(format!(
                        "{expr_format} is out of bands"
                    )));
                }

                if let Some(ts) = from_timestamp.filter(|t| *t != 0.0) {
                    low_v = low_from_current_date_number(field_index, step, ts as i64)?;
                }

                let rng: Vec<i64> = if low_v > high_v {
                    // Backtracking range: Sat-Sun, Apr-Jan, ...
                    let whole_len = hi_r - lo_r + 1;
                    let mut rng: Vec<i64> = (low_v..=hi_r).step_by(step as usize).collect();
                    let mut to_skip = 0i64;
                    if let Some(&last) = rng.last() {
                        let already_skipped = hi_r - last;
                        let curpos = last - lo_r;
                        if (curpos + step) > whole_len && already_skipped < step {
                            to_skip = step - already_skipped;
                        }
                    }
                    if lo_r + to_skip <= high_v {
                        rng.extend((lo_r + to_skip..=high_v).step_by(step as usize));
                    }
                    rng
                } else if low_v == high_v {
                    // Jan-Jan / Sun-Sun means the whole cycle
                    (lo_r..=hi_r).step_by(step as usize).collect()
                } else {
                    (low_v..=high_v).step_by(step as usize).collect()
                };

                let rendered: Vec<String> = if field_index == DOW_FIELD {
                    match nth {
                        Some(Nth::Num(n)) => rng.iter().map(|i| format!("{i}#{n}")).collect(),
                        _ => rng.iter().map(|i| i.to_string()).collect(),
                    }
                } else {
                    rng.iter().map(|i| i.to_string()).collect()
                };

                for item in &rendered {
                    if !seen.contains(item) {
                        e_list.push(item.clone());
                    }
                }
                seen.extend(rendered);
            } else {
                if t.starts_with('-') {
                    return Err(CroniterError::BadCron(format!(
                        "[{expr_format}] is not acceptable, negative numbers not allowed"
                    )));
                }
                let mut item = if STAR_OR_INT_RE.is_match(&t) {
                    if t == "*" {
                        Item::Star
                    } else {
                        Item::Num(t.parse::<i64>().map_err(|_| {
                            CroniterError::BadCron(format!("[{expr_format}] is not acceptable"))
                        })?)
                    }
                } else {
                    alphaconv(field_index, &t, &expressions)?
                };

                item = value_alias(item, field_index, len_expressions);

                if let Item::Num(n) = item {
                    if n < lo_r || n > hi_r {
                        return Err(CroniterError::BadCron(format!(
                            "[{expr_format}] is not acceptable, out of range"
                        )));
                    }
                }

                res.push(item);

                if field_index == DOW_FIELD {
                    if let Some(n) = nth {
                        nth_weekday_of_month.entry(item).or_default().insert(n);
                    }
                }
            }
        }

        let mut res: Vec<Item> = res
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();

        if res.len() == LEN_MEANS_ALL[field_index] {
            // Make sure the wildcard is used in the correct way
            // (avoid over-optimization).
            let keep_explicit = (field_index == DAY_FIELD && !expressions[DOW_FIELD].contains('*'))
                || (field_index == DOW_FIELD && !expressions[DAY_FIELD].contains('*'));
            if !keep_explicit {
                res = vec![Item::Star];
            }
        }

        expanded.push(if res.len() == 1 && res[0] == Item::Star {
            vec![Item::Star]
        } else {
            res
        });
    }

    // Check that the dow combo in use is supported (croniter.py:1190).
    if !nth_weekday_of_month.is_empty() {
        let dow_set: BTreeSet<Item> = expanded[DOW_FIELD].iter().copied().collect();
        let mut diff: BTreeSet<Item> = dow_set
            .difference(&nth_weekday_of_month.keys().copied().collect())
            .copied()
            .collect();
        diff.remove(&Item::Star);
        if !diff.is_empty() && dow_set.len() != LEN_MEANS_ALL[DOW_FIELD] {
            return Err(CroniterError::UnsupportedSyntax(format!(
                "day-of-week field does not support mixing literal values and nth day of week syntax.  Cron: '{expr_format}'    dow={diff:?} vs nth={nth_weekday_of_month:?}"
            )));
        }
    }

    if strict {
        check_strict(
            expr_format,
            &expanded,
            strict_year,
            &mut nth_weekday_of_month.keys().copied(),
        )?;
    }

    Ok(Expanded {
        expanded,
        nth_weekday_of_month,
        expressions,
        nearest_weekday,
    })
}

/// Cross-validate day-of-month against month (croniter.py:1202).
fn check_strict(
    expr_format: &str,
    expanded: &[Vec<Item>],
    strict_year: Option<&[i64]>,
    _nth: &mut dyn Iterator<Item = Item>,
) -> Result<()> {
    let days = &expanded[DAY_FIELD];
    let months = &expanded[MONTH_FIELD];

    let days_is_star = days.len() == 1 && days[0] == Item::Star;
    let days_is_last = days.len() == 1 && days[0] == Item::Last;
    let months_is_star = months.len() == 1 && months[0] == Item::Star;
    if days_is_star || days_is_last || months_is_star {
        return Ok(());
    }

    let int_days: Vec<i64> = days.iter().filter_map(Item::as_num).collect();
    let int_months: Vec<i64> = months.iter().filter_map(Item::as_num).collect();
    if int_days.is_empty() || int_months.is_empty() {
        return Ok(());
    }

    let mut days_in_month: BTreeMap<i64, i64> =
        (1..=12).map(|m| (m, DAYS[(m - 1) as usize])).collect();

    if int_months.contains(&2) {
        let has_leap_year = if let Some(years) = strict_year {
            years.iter().any(|y| is_leap(*y))
        } else if expanded.len() > YEAR_FIELD {
            let years = &expanded[YEAR_FIELD];
            if years.len() == 1 && years[0] == Item::Star {
                true
            } else {
                let int_years: Vec<i64> = years.iter().filter_map(Item::as_num).collect();
                if int_years.is_empty() {
                    true
                } else {
                    int_years.iter().any(|y| is_leap(*y))
                }
            }
        } else {
            true
        };
        if has_leap_year {
            days_in_month.insert(2, 29);
        }
    }

    let min_day = *int_days.iter().min().expect("non-empty");
    let max_possible = int_months
        .iter()
        .map(|m| days_in_month[m])
        .max()
        .expect("non-empty");
    if min_day > max_possible {
        return Err(CroniterError::BadCron(format!(
            "[{expr_format}] is not acceptable. Day(s) {int_days:?} can never occur in month(s) {int_months:?}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ex(s: &str) -> Vec<Vec<Item>> {
        expand(s, None, false, None, false, None).unwrap().expanded
    }

    #[test]
    fn every_minute_is_all_stars() {
        assert_eq!(ex("* * * * *"), vec![vec![Item::Star]; 5]);
    }

    #[test]
    fn on_the_hour() {
        let e = ex("0 0 * * *");
        assert_eq!(e[0], vec![Item::Num(0)]);
        assert_eq!(e[1], vec![Item::Num(0)]);
        assert_eq!(e[2], vec![Item::Star]);
    }

    #[test]
    fn list_and_range_with_alpha_dow() {
        let e = ex("0-5,10 * * * mon-fri");
        assert_eq!(
            e[0],
            vec![0, 1, 2, 3, 4, 5, 10]
                .into_iter()
                .map(Item::Num)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            e[4],
            vec![1, 2, 3, 4, 5]
                .into_iter()
                .map(Item::Num)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn nth_weekday_is_lifted_out_of_the_dow_field() {
        let r = expand("* * * * 2#3", None, false, None, false, None).unwrap();
        assert_eq!(r.expanded[4], vec![Item::Num(2)]);
        assert_eq!(
            r.nth_weekday_of_month.get(&Item::Num(2)),
            Some(&[Nth::Num(3)].into_iter().collect::<BTreeSet<_>>())
        );
    }

    #[test]
    fn last_day_of_month_marker() {
        let e = ex("0 * l * *");
        assert_eq!(e[2], vec![Item::Last]);
    }

    #[test]
    fn seconds_field_step() {
        let e = ex("0 0 * * * */15");
        assert_eq!(
            e[5],
            vec![0, 15, 30, 45]
                .into_iter()
                .map(Item::Num)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn wrong_column_count_is_rejected() {
        assert!(expand("* * * *", None, false, None, false, None).is_err());
        assert!(expand("* * * * * * * *", None, false, None, false, None).is_err());
    }

    #[test]
    fn out_of_range_is_rejected() {
        assert!(expand("60 * * * *", None, false, None, false, None).is_err());
        assert!(expand("* 24 * * *", None, false, None, false, None).is_err());
        assert!(expand("* * 32 * *", None, false, None, false, None).is_err());
        assert!(expand("* * * 13 *", None, false, None, false, None).is_err());
    }

    #[test]
    fn zero_step_is_rejected() {
        assert!(expand("*/0 * * * *", None, false, None, false, None).is_err());
    }

    #[test]
    fn bad_alpha_is_not_alpha_error() {
        let err = expand("* * * * xyz", None, false, None, false, None).unwrap_err();
        assert!(matches!(err, CroniterError::NotAlpha(_)));
    }

    #[test]
    fn dow_seven_normalises_to_zero_on_unix_cron() {
        let e = ex("0 0 * * 7");
        assert_eq!(e[4], vec![Item::Num(0)]);
    }

    #[test]
    fn nearest_weekday_is_lifted_out() {
        let r = expand("0 9 15w * *", None, false, None, false, None).unwrap();
        assert!(r.nearest_weekday.contains(&15));
    }

    #[test]
    fn nearest_weekday_rejects_lists() {
        assert!(expand("0 9 1,15w * *", None, false, None, false, None).is_err());
    }

    #[test]
    fn item_ordering_matches_python_string_sort() {
        let mut v = vec![Item::Last, Item::Num(5), Item::Star, Item::Num(1)];
        v.sort();
        assert_eq!(v, vec![Item::Star, Item::Num(1), Item::Num(5), Item::Last]);
    }
}
