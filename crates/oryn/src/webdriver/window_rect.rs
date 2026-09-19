//! Headless window rectangles: the native viewport is the window's drawable area.
//! The native platform owns virtual screen coordinates independently of viewport size.
use super::CommandError;
use oryn_common::v4::ViewportState;
use oryn_native::PageHandle;
use serde_json::{Value, json};

#[derive(Default, Debug, PartialEq)]
struct RequestedRect {
    x: Option<f64>,
    y: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
}
impl RequestedRect {
    fn parse(value: &Value) -> Result<Self, CommandError> {
        let fields = value
            .as_object()
            .ok_or_else(|| CommandError::invalid("window rectangle must be an object"))?;
        let number = |name: &str, min: f64| -> Result<Option<f64>, CommandError> {
            match fields.get(name) {
                None | Some(Value::Null) => Ok(None),
                Some(value) => value
                    .as_f64()
                    .filter(|value| value.is_finite() && *value >= min && *value <= 2_147_483_647.)
                    .map(Some)
                    .ok_or_else(|| {
                        CommandError::invalid(format!("invalid window rectangle {name}"))
                    }),
            }
        };
        Ok(Self {
            x: number("x", -2_147_483_648.)?,
            y: number("y", -2_147_483_648.)?,
            width: number("width", 0.)?,
            height: number("height", 0.)?,
        })
    }
}
fn response(viewport: ViewportState, position: (i32, i32)) -> Value {
    json!({"x":position.0,"y":position.1,"width":viewport.width,"height":viewport.height})
}
fn native_error(error: oryn_native::RuntimeError) -> CommandError {
    CommandError {
        code: if matches!(
            &error,
            oryn_native::RuntimeError::Closed | oryn_native::RuntimeError::PageNotFound(_)
        ) {
            "no such window"
        } else {
            "unknown error"
        },
        message: error.to_string(),
    }
}
pub async fn get(
    page: &PageHandle,
    context: &oryn_native::BrowserContext,
) -> Result<Value, CommandError> {
    let viewport = page.paint_snapshot().await.map_err(native_error)?.viewport;
    Ok(response(
        viewport,
        context
            .webdriver_window_position(page.page_id())
            .map_err(native_error)?,
    ))
}
pub fn validate(value: &Value) -> Result<(), CommandError> {
    RequestedRect::parse(value).map(|_| ())
}
pub async fn set(
    page: &PageHandle,
    context: &oryn_native::BrowserContext,
    value: &Value,
) -> Result<Value, CommandError> {
    let requested = RequestedRect::parse(value)?;
    let mut viewport = page.paint_snapshot().await.map_err(native_error)?.viewport;
    // WebDriver updates size only when both dimensions are supplied. Native
    // windows have a one-pixel minimum; report the actual applied dimensions.
    if let (Some(width), Some(height)) = (requested.width, requested.height) {
        viewport.width = width.trunc().max(1.);
        viewport.height = height.trunc().max(1.);
        viewport = page.set_viewport(viewport).await.map_err(native_error)?;
    }
    if let (Some(x), Some(y)) = (requested.x, requested.y) {
        context
            .webdriver_move_window(page.page_id(), x.trunc() as i32, y.trunc() as i32)
            .map_err(native_error)?;
    }
    Ok(response(
        viewport,
        context
            .webdriver_window_position(page.page_id())
            .map_err(native_error)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rectangle_conversion_validates_all_fields_before_mutation() {
        for value in [
            json!({"width":30,"height":20,"x":false}),
            json!({"height":-1}),
            json!({"x":-2_147_483_649i64}),
            json!({"width":2_147_483_648u64}),
            json!({"y":"5"}),
        ] {
            assert!(RequestedRect::parse(&value).is_err());
        }
        let value =
            RequestedRect::parse(&json!({"width":12.5,"height":null,"x":-5,"ignored":true}))
                .unwrap();
        assert_eq!(value.width, Some(12.5));
        assert_eq!(value.height, None);
        assert_eq!(value.x, Some(-5.));
    }
    #[tokio::test]
    async fn size_changes_preserve_device_scale_and_partial_size_is_unchanged() {
        let browser = oryn_native::Browser::new();
        let context = browser.new_audited_context_with_isolated_auxiliaries(
            oryn_native::ContextOptions::loopback_test(),
        );
        let page = context.new_page();
        page.goto("about:blank").await.unwrap();
        page.set_viewport(ViewportState {
            width: 80.,
            height: 60.,
            device_pixel_ratio: 2.,
        })
        .await
        .unwrap();
        assert_eq!(
            set(&page, &context, &json!({"width":30})).await.unwrap()["width"],
            80.
        );
        assert!(
            set(&page, &context, &json!({"width":30,"height":20,"x":false}))
                .await
                .is_err()
        );
        assert_eq!(get(&page, &context).await.unwrap()["width"], 80.);
        assert_eq!(
            set(&page, &context, &json!({"width":30,"height":20}))
                .await
                .unwrap(),
            json!({"x":0,"y":0,"width":30.,"height":20.})
        );
        assert_eq!(
            page.paint_snapshot()
                .await
                .unwrap()
                .viewport
                .device_pixel_ratio,
            2.
        );
        assert_eq!(
            set(&page, &context, &json!({"x":100,"y":200}))
                .await
                .unwrap(),
            json!({"x":100,"y":200,"width":30.,"height":20.})
        );
        assert_eq!(
            set(&page, &context, &json!({"x":400})).await.unwrap()["x"],
            100
        );
        assert!(
            set(&page, &context, &json!({"x":900,"y":false}))
                .await
                .is_err()
        );
        assert_eq!(get(&page, &context).await.unwrap()["x"], 100);

        context.close_page(page.page_id()).await.unwrap();
        context.retire_workers().await.unwrap();
    }
}
