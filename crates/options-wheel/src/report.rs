// report.rs — --report flag output renderer (TNA-30)

use anyhow::Result;

use crate::performance::{open_db, query_recent_cycles, query_symbol_summary, CycleRow, SymbolSummaryRow};

// ─── Box-drawing helpers ──────────────────────────────────────────────────────

fn row_separator(widths: &[usize], left: &str, mid: &str, right: &str, fill: &str) -> String {
    let inner: Vec<String> = widths.iter().map(|&w| fill.repeat(w + 2)).collect();
    format!("{}{}{}", left, inner.join(mid), right)
}

fn data_row(cells: &[String], widths: &[usize]) -> String {
    let inner: Vec<String> = cells
        .iter()
        .zip(widths.iter())
        .map(|(cell, &w)| format!(" {:<width$} ", cell, width = w))
        .collect();
    format!("│{}│", inner.join("│"))
}

fn right_data_row(cells: &[String], widths: &[usize]) -> String {
    // First cell left-aligned (symbol), rest right-aligned (numbers)
    let inner: Vec<String> = cells
        .iter()
        .zip(widths.iter())
        .enumerate()
        .map(|(i, (cell, &w))| {
            if i == 0 {
                format!(" {:<width$} ", cell, width = w)
            } else {
                format!(" {:>width$} ", cell, width = w)
            }
        })
        .collect();
    format!("│{}│", inner.join("│"))
}

// ─── Symbol summary table ─────────────────────────────────────────────────────

fn print_symbol_table(rows: &[SymbolSummaryRow]) {
    let headers = [
        "Underlying",
        "Cycles",
        "Total Premium $",
        "Avg P&L / Cycle",
        "Win Rate %",
        "Avg Days/Cycle",
        "Annualized Yield",
    ];

    // Compute column widths (header vs data)
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();

    let data_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.underlying.clone(),
                r.cycles.to_string(),
                format!("{:.2}", r.total_premium),
                format!("{:.2}", r.avg_pnl_per_cycle),
                format!("{:.1}%", r.win_rate_pct),
                format!("{:.1}", r.avg_cycle_days),
                format!("{:.1}%", r.annualized_yield_pct),
            ]
        })
        .collect();

    for row in &data_rows {
        for (i, cell) in row.iter().enumerate() {
            if cell.len() > widths[i] {
                widths[i] = cell.len();
            }
        }
    }

    let top = row_separator(&widths, "┌", "┬", "┐", "─");
    let mid = row_separator(&widths, "├", "┼", "┤", "─");
    let bot = row_separator(&widths, "└", "┴", "┘", "─");

    let header_cells: Vec<String> = headers.iter().map(|h| h.to_string()).collect();

    println!("{}", top);
    println!("{}", data_row(&header_cells, &widths));
    println!("{}", mid);

    if data_rows.is_empty() {
        println!("  No completed cycles yet.");
    } else {
        for row in &data_rows {
            println!("{}", right_data_row(row, &widths));
        }
    }

    println!("{}", bot);
}

// ─── Recent cycles table ──────────────────────────────────────────────────────

fn print_cycles_table(rows: &[CycleRow]) {
    let headers = [
        "Underlying",
        "Cycle Ended",
        "Total Premium $",
        "Realized P&L $",
        "CSP Outcome",
        "Days",
    ];

    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();

    let data_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.underlying.clone(),
                r.cycle_ended.clone(),
                format!("{:.2}", r.total_premium),
                format!("{:+.2}", r.realized_pnl),
                r.csp_outcome.clone(),
                format!("{:.0}", r.cycle_days),
            ]
        })
        .collect();

    for row in &data_rows {
        for (i, cell) in row.iter().enumerate() {
            if cell.len() > widths[i] {
                widths[i] = cell.len();
            }
        }
    }

    let top = row_separator(&widths, "┌", "┬", "┐", "─");
    let mid = row_separator(&widths, "├", "┼", "┤", "─");
    let bot = row_separator(&widths, "└", "┴", "┘", "─");

    let header_cells: Vec<String> = headers.iter().map(|h| h.to_string()).collect();

    println!("{}", top);
    println!("{}", data_row(&header_cells, &widths));
    println!("{}", mid);

    if data_rows.is_empty() {
        println!("  No completed cycles yet.");
    } else {
        for row in &data_rows {
            println!("{}", right_data_row(row, &widths));
        }
    }

    println!("{}", bot);
}

// ─── Public entry point ───────────────────────────────────────────────────────

/// Open the performance DB, run queries, and print the full report to stdout.
pub fn print_report() -> Result<()> {
    let conn = open_db()?;
    let today = chrono::Local::now().date_naive();

    println!("== Wheel Strategy Performance Report ==");
    println!("Generated: {}", today.format("%Y-%m-%d"));
    println!();

    let summary = query_symbol_summary(&conn)?;

    println!("By Symbol:");
    if summary.is_empty() {
        println!("  No completed cycles yet.");
    } else {
        print_symbol_table(&summary);
    }

    println!();
    println!("All Cycles (last 10):");

    let cycles = query_recent_cycles(&conn, 10)?;
    if cycles.is_empty() {
        println!("  No completed cycles yet.");
    } else {
        print_cycles_table(&cycles);
    }

    Ok(())
}
