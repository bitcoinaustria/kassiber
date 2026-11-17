//! The local zone's UTC offset at an instant, as the webview's `Date` applied it, from the C
//! library: `localtime_r` and `tm_gmtoff` on Unix, `_localtime64_s` on Windows (which has
//! neither). No process, no time zone database of our own.

/// Seconds east of UTC at Unix time `t`, or 0 when the C library can't tell.
pub fn offset_secs(t: i64) -> i64 {
    imp::offset(t)
}

/// The local (year, month 1–12) at Unix time `t`.
pub fn year_month(t: i64) -> (i32, u32) {
    let local = t + offset_secs(t);
    let d = crate::format::Day::from_epoch_days(local.div_euclid(86_400));
    (d.y, d.m)
}

#[cfg(unix)]
mod imp {
    #[repr(C)]
    struct Tm {
        tm_sec: i32,
        tm_min: i32,
        tm_hour: i32,
        tm_mday: i32,
        tm_mon: i32,
        tm_year: i32,
        tm_wday: i32,
        tm_yday: i32,
        tm_isdst: i32,
        tm_gmtoff: std::ffi::c_long,
        tm_zone: *const std::ffi::c_char,
    }

    unsafe extern "C" {
        fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
    }

    pub fn offset(t: i64) -> i64 {
        let mut tm = Tm {
            tm_sec: 0,
            tm_min: 0,
            tm_hour: 0,
            tm_mday: 0,
            tm_mon: 0,
            tm_year: 0,
            tm_wday: 0,
            tm_yday: 0,
            tm_isdst: 0,
            tm_gmtoff: 0,
            tm_zone: std::ptr::null(),
        };
        // SAFETY: `localtime_r` writes only into `tm`, a properly laid-out `struct tm`.
        let ok = unsafe { !localtime_r(&t, &mut tm).is_null() };
        if ok { tm.tm_gmtoff as i64 } else { 0 }
    }
}

#[cfg(windows)]
mod imp {
    /// The UCRT's `struct tm`: nine ints, no `tm_gmtoff`.
    #[repr(C)]
    #[derive(Default)]
    struct Tm {
        tm_sec: i32,
        tm_min: i32,
        tm_hour: i32,
        tm_mday: i32,
        tm_mon: i32,
        tm_year: i32,
        tm_wday: i32,
        tm_yday: i32,
        tm_isdst: i32,
    }

    unsafe extern "C" {
        fn _localtime64_s(out: *mut Tm, t: *const i64) -> i32;
    }

    /// The local wall-clock time read back as if it were UTC, minus the instant.
    pub fn offset(t: i64) -> i64 {
        let mut tm = Tm::default();
        // SAFETY: `_localtime64_s` writes only into `tm`, a properly laid-out `struct tm`.
        if unsafe { _localtime64_s(&mut tm, &t) } != 0 {
            return 0;
        }
        let days = crate::format::Day {
            y: tm.tm_year + 1900,
            m: (tm.tm_mon + 1) as u32,
            d: tm.tm_mday as u32,
        }
        .epoch_days();
        days * 86_400
            + i64::from(tm.tm_hour) * 3600
            + i64::from(tm.tm_min) * 60
            + i64::from(tm.tm_sec)
            - t
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    pub fn offset(_t: i64) -> i64 {
        0
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn offsets_are_whole_minutes_within_a_day() {
        for t in [0, 1_700_000_000, 1_782_000_000] {
            let o = super::offset_secs(t);
            assert!(o.abs() < 86_400 && o % 60 == 0, "{o}");
            let (y, m) = super::year_month(t);
            assert!((1969..=2027).contains(&y) && (1..=12).contains(&m));
        }
    }
}
