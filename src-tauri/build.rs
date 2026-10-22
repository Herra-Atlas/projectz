fn main() {
    // A version alone does not say which build someone is running: `0.5.0` can be
    // a dozen different binaries, and the System page is where that question gets
    // asked. The date is baked in at compile time rather than read from the file
    // system, because an installed copy's timestamp is when it was written, not
    // when it was made.
    println!("cargo:rustc-env=BUILD_DATE={}", today());
    tauri_build::build();
}

/// Today, as `YYYY-MM-DD`, from the system clock.
///
/// Howard Hinnant's civil-from-days, which is exact over the range a build date
/// can plausibly fall in. Written out rather than pulled in as a build
/// dependency: it is fifteen lines and one date.
fn today() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0) as i64;
    let days = seconds.div_euclid(86_400);

    let shifted = days + 719_468;
    let era = if shifted >= 0 { shifted } else { shifted - 146_096 } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_position = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_position + 2) / 5 + 1;
    let month = if month_position < 10 {
        month_position + 3
    } else {
        month_position - 9
    };
    let year = if month <= 2 { year + 1 } else { year };

    format!("{year:04}-{month:02}-{day:02}")
}
