//! Rich terminal output using ANSI escape codes.
//!
//! No external colour crates are required — all sequences are inlined.  The
//! functions here write directly to **stdout** (or **stderr** for errors).

// ANSI colour codes.
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const BLUE: &str = "\x1b[34m";

/// Print a bold cyan section header to stdout.
///
/// Suitable for labelling major sections of command output.
///
/// # Example
/// ```
/// truenorth_cli::output::terminal::print_header("TrueNorth");
/// ```
pub fn print_header(text: &str) {
    println!("{BOLD}{CYAN}{text}{RESET}");
}

/// Print a green success message to stdout.
pub fn print_success(text: &str) {
    println!("{GREEN}✓{RESET} {text}");
}

/// Print a red error message to **stderr**.
pub fn print_error(text: &str) {
    eprintln!("{RED}✗{RESET} {BOLD}{text}{RESET}");
}

/// Print a blue informational message to stdout.
pub fn print_info(text: &str) {
    println!("{BLUE}ℹ{RESET} {text}");
}

/// Print a yellow warning message to stdout.
pub fn print_warning(text: &str) {
    println!("{YELLOW}⚠{RESET} {text}");
}

/// Print a simple ASCII table to stdout.
///
/// `headers` is the list of column titles; each element of `rows` must have
/// the same length as `headers`.  Column widths are computed automatically
/// from the maximum content width.
///
/// # Example
///
/// ```
/// truenorth_cli::output::terminal::print_table(
///     &["Name", "Status"],
///     &[vec!["my-skill".to_string(), "installed".to_string()]],
/// );
/// ```
pub fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    if headers.is_empty() {
        return;
    }

    // Calculate column widths.
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(cell.len());
            }
        }
    }

    // Build separator line.
    let sep: String = widths
        .iter()
        .map(|w| "-".repeat(w + 2))
        .collect::<Vec<_>>()
        .join("+");
    let sep = format!("+{sep}+");

    println!("{sep}");

    // Header row.
    let header_cells: String = headers
        .iter()
        .zip(widths.iter())
        .map(|(h, w)| format!(" {BOLD}{CYAN}{h:<w$}{RESET} ", w = w))
        .collect::<Vec<_>>()
        .join("|");
    println!("|{header_cells}|");

    println!("{sep}");

    // Data rows.
    for row in rows {
        let cells: String = row
            .iter()
            .zip(widths.iter())
            .map(|(c, w)| format!(" {c:<w$} ", w = w))
            .collect::<Vec<_>>()
            .join("|");
        println!("|{cells}|");
    }

    println!("{sep}");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test — ensure the functions run without panicking.
    #[test]
    fn smoke_test_output_functions() {
        // These write to stdout/stderr; we just assert they don't panic.
        print_header("Test Header");
        print_success("Operation completed");
        print_error("Something went wrong");
        print_info("Informational note");
        print_warning("Heads-up");
    }

    #[test]
    fn smoke_test_print_table() {
        print_table(
            &["Name", "Version", "Status"],
            &[
                vec![
                    "truenorth".to_string(),
                    "0.1.0".to_string(),
                    "ok".to_string(),
                ],
                vec![
                    "truenorth-core".to_string(),
                    "0.1.0".to_string(),
                    "ok".to_string(),
                ],
            ],
        );
    }

    #[test]
    fn print_table_empty_rows() {
        // Should not panic on empty row set.
        print_table(&["Col A", "Col B"], &[]);
    }
}
