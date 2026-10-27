use raytrace_core::{
    Error,
    math::V3,
    model::{Aperture, Interaction, Material, Mode, Ray, Scene, Settings, Shape, Surface},
};
use raytrace_io::zemax::read_zmx;

fn scene() -> Scene {
    Scene {
        name: "エラーテスト".into(),
        reference_plane: "発射面".into(),
        mode: Mode::Sequential,
        materials: vec![Material::Constant {
            n: 1.0,
            absorption_per_mm: 0.0,
        }],
        surfaces: vec![Surface {
            name: "検出面".into(),
            coating: None,
            pose: Default::default(),
            shape: Shape::Plane,
            aperture: Aperture::Circle { radius_mm: 1.0 },
            interaction: Interaction::Detector { pixels: [1, 1] },
        }],
        settings: Settings::default(),
        sources: vec![],
    }
}

fn ray() -> Ray {
    Ray {
        id: 42,
        position_mm: V3([0., 0., -1.]),
        direction: V3([0., 0., 1.]),
        weight: 1.,
        wavelength_um: 0.55,
        medium: 0,
        optical_path_mm: 0.,
    }
}

#[test]
fn duplicate_surface_name_is_an_invalid_scene_in_japanese() {
    let mut scene = scene();
    scene.surfaces.push(scene.surfaces[0].clone());
    let error = scene.prepare().unwrap_err();
    assert!(matches!(error, Error::InvalidScene(_)));
    assert_eq!(error.to_string(), "面の名前が重複しています");
}

#[test]
fn missing_zmx_retains_path_and_io_source() {
    let path = std::env::temp_dir()
        .join(format!("raytrace-missing-zmx-{}", std::process::id()))
        .join("missing.zmx");
    let error = read_zmx(&path, &[]).unwrap_err();
    match &error {
        raytrace_io::Error::Io {
            path: actual,
            source,
        } => {
            assert_eq!(actual, &path);
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("unexpected error: {other:?}"),
    }
    assert!(std::error::Error::source(&error).is_some());
    assert!(!error.to_string().is_ascii());
}

#[test]
fn catalog_wavelength_error_retains_material_wavelength_and_launch() {
    let mut scene = scene();
    scene.materials.push(Material::Catalog {
        formula: 1,
        coefficients: vec![2.25, 0., 0., 0., 0., 0.],
        valid_range_um: [0.4, 0.8],
        absorption_per_mm: 0.,
    });
    let scene = scene.prepare().unwrap();
    let mut ray = ray();
    ray.wavelength_um = 0.9;
    let error = ray.validate(&scene).unwrap_err();
    assert!(matches!(
        error,
        Error::Wavelength {
            material: 1,
            wavelength_um: 0.9,
            launch: 42
        }
    ));
    assert_eq!(
        error.to_string(),
        "材料 1 は波長 0.9 um（光線 42）で定義されていません。有効な波長範囲を確認してください"
    );
}

#[test]
fn library_errors_are_thread_safe_standard_errors() {
    fn check<T: std::error::Error + Send + Sync + 'static>() {}
    check::<Error>();
    check::<raytrace_io::Error>();
    check::<raytrace_app::Error>();
    check::<raytrace_rs::error::CoreError>();
    check::<raytrace_rs::error::IoError>();
    check::<raytrace_rs::error::AppError>();
}

#[test]
fn translated_validation_paths_always_display_japanese() {
    let mut errors = Vec::new();
    let mut invalid = scene();
    invalid.materials.clear();
    errors.push(invalid.prepare().unwrap_err());
    let mut invalid = scene();
    invalid.settings.max_depth = 0;
    errors.push(invalid.prepare().unwrap_err());
    let mut invalid = scene();
    invalid.materials[0] = Material::Constant {
        n: -1.,
        absorption_per_mm: 0.,
    };
    errors.push(invalid.prepare().unwrap_err());
    let mut invalid = scene();
    invalid.surfaces[0].pose.translation_mm = V3([f64::NAN, 0., 0.]);
    errors.push(invalid.prepare().unwrap_err());
    let mut invalid = scene();
    invalid.surfaces[0].aperture = Aperture::Circle { radius_mm: 0. };
    errors.push(invalid.prepare().unwrap_err());
    let mut invalid = scene();
    invalid.surfaces[0].shape = Shape::Sphere { radius_mm: 0.5 };
    errors.push(invalid.prepare().unwrap_err());
    let mut invalid = scene();
    invalid.surfaces[0].interaction = Interaction::Dielectric {
        minus_medium: 0,
        plus_medium: 1,
        reflectance: None,
    };
    errors.push(invalid.prepare().unwrap_err());
    let mut invalid = scene();
    invalid.surfaces[0].interaction = Interaction::Mirror { reflectance: 2. };
    errors.push(invalid.prepare().unwrap_err());
    let mut invalid = scene();
    invalid.surfaces[0].interaction = Interaction::Detector { pixels: [0, 1] };
    errors.push(invalid.prepare().unwrap_err());
    for pixels in [[1001, 1000], [usize::MAX, 2]] {
        let mut invalid = scene();
        invalid.surfaces[0].interaction = Interaction::Detector { pixels };
        let error = invalid.prepare().unwrap_err();
        assert!(matches!(error, Error::Limit(_)));
        errors.push(error);
    }
    let prepared = scene().prepare().unwrap();
    let mut ray = ray();
    ray.direction = V3([0.; 3]);
    let error = ray.validate(&prepared).unwrap_err();
    assert!(matches!(error, Error::InvalidRay { id: 42, .. }));
    errors.push(error);
    for error in errors {
        assert!(matches!(
            error,
            Error::InvalidScene(_) | Error::InvalidRay { .. } | Error::Limit(_)
        ));
        assert!(!error.to_string().is_ascii(), "{error:?}");
    }
}
