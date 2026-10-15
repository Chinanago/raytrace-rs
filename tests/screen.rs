use raytrace_rs::{screen::ScreenAnalysis, trace::DetectorMap};

fn map(nx: usize, ny: usize, hx: f64, hy: f64, f: impl Fn(f64, f64) -> f64) -> DetectorMap {
    let mut weights = Vec::new();
    for y in 0..ny {
        for x in 0..nx {
            weights.push(f(
                -hx + (x as f64 + 0.5) * 2.0 * hx / nx as f64,
                -hy + (y as f64 + 0.5) * 2.0 * hy / ny as f64,
            ));
        }
    }
    DetectorMap {
        name: "Synthetic screen".into(),
        pixels: [nx, ny],
        half_extent_mm: [hx, hy],
        weights,
    }
}

#[test]
fn elliptical_gaussian_slices_use_physical_pixel_centres_and_half_height() {
    let (sigma_x, sigma_y) = (0.8, 1.7);
    let detector = map(401, 501, 4.01, 7.515, |x, y| {
        (-0.5 * ((x / sigma_x).powi(2) + (y / sigma_y).powi(2))).exp()
    });
    let original = detector.weights.clone();
    let a = ScreenAnalysis::from_detector(&detector).unwrap();
    let factor = 2.0 * (2.0 * std::f64::consts::LN_2).sqrt();
    assert_eq!(a.peak_pixel, Some([200, 250]));
    let p = a.peak_position_mm.unwrap();
    assert!(p[0].abs() < 1e-12 && p[1].abs() < 1e-12);
    for (profile, sigma) in [
        (a.x_profile.unwrap(), sigma_x),
        (a.y_profile.unwrap(), sigma_y),
    ] {
        assert!((profile.half_maximum.width_mm().unwrap() - factor * sigma).abs() < 0.0002);
        assert!(!profile.half_maximum.multiple_lobes);
    }
    assert_eq!(detector.weights, original);
}

#[test]
fn clipped_and_flat_screens_keep_missing_crossings_unknown() {
    let detector = map(11, 9, 0.55, 0.45, |x, y| {
        (-0.5 * (((x + 0.5) / 0.3).powi(2) + (y / 0.1).powi(2))).exp()
    });
    let a = ScreenAnalysis::from_detector(&detector).unwrap();
    let x = a.x_profile.unwrap().half_maximum;
    assert_eq!(x.lower_mm, None);
    assert!(x.upper_mm.is_some());
    assert_eq!(x.width_mm(), None);
    assert!(a.y_profile.unwrap().half_maximum.width_mm().is_some());
    let flat = ScreenAnalysis::from_detector(&map(5, 5, 1.0, 1.0, |_, _| 1.0)).unwrap();
    assert_eq!(flat.x_profile.unwrap().half_maximum.width_mm(), None);
    assert_eq!(flat.y_profile.unwrap().half_maximum.width_mm(), None);
}

#[test]
fn separate_peaks_report_the_selected_lobe_and_first_peak_tie() {
    let detector = DetectorMap {
        name: "Two peaks".into(),
        pixels: [9, 3],
        half_extent_mm: [4.5, 1.5],
        weights: [
            vec![0.0; 9],
            vec![0.0, 1.0, 2.0, 1.0, 0.0, 1.0, 2.0, 1.0, 0.0],
            vec![0.0; 9],
        ]
        .concat(),
    };
    let a = ScreenAnalysis::from_detector(&detector).unwrap();
    assert_eq!(a.peak_pixel, Some([2, 1]));
    let x = a.x_profile.unwrap().half_maximum;
    assert!(x.multiple_lobes);
    assert_eq!(x.lower_mm, Some(-3.0));
    assert_eq!(x.upper_mm, Some(-1.0));
    assert_eq!(x.width_mm(), Some(2.0));
}

#[test]
fn zero_single_pixel_and_invalid_maps_do_not_invent_a_width() {
    let zero = ScreenAnalysis::from_detector(&map(3, 3, 1.0, 2.0, |_, _| 0.0)).unwrap();
    assert_eq!(zero.peak_weight, 0.0);
    assert_eq!(zero.peak_position_mm, None);
    assert!(zero.x_profile.is_none() && zero.y_profile.is_none());
    let single = ScreenAnalysis::from_detector(&map(1, 1, 1.0, 2.0, |_, _| 2.0)).unwrap();
    assert_eq!(single.x_profile.unwrap().half_maximum.width_mm(), None);
    assert_eq!(single.y_profile.unwrap().half_maximum.width_mm(), None);
    let mut invalid = map(3, 3, 1.0, 2.0, |_, _| 1.0);
    invalid.weights[0] = f64::NAN;
    assert!(ScreenAnalysis::from_detector(&invalid).is_err());
    invalid.weights[0] = -1.0;
    assert!(ScreenAnalysis::from_detector(&invalid).is_err());
}

#[test]
fn scaling_power_changes_the_peak_and_preserves_half_width() {
    let first = map(101, 81, 5.05, 4.05, |x, y| (-0.5 * (x * x + y * y)).exp());
    let mut scaled = first.clone();
    for w in &mut scaled.weights {
        *w *= 37.0;
    }
    let a = ScreenAnalysis::from_detector(&first).unwrap();
    let b = ScreenAnalysis::from_detector(&scaled).unwrap();
    assert_eq!(b.peak_weight, 37.0 * a.peak_weight);
    for (a, b) in [
        (a.x_profile.unwrap(), b.x_profile.unwrap()),
        (a.y_profile.unwrap(), b.y_profile.unwrap()),
    ] {
        assert!(
            (a.half_maximum.width_mm().unwrap() - b.half_maximum.width_mm().unwrap()).abs() < 1e-12
        );
    }
}
