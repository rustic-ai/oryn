//! Image-map areas have no layout box. Resolve their regions through the painted
//! image's content coordinates before event and CSSOM hit-test retargeting.
use super::*;

fn numbers(value: &str) -> Vec<f64> {
    value
        .split(|c: char| matches!(c, ',' | ';' | '\t' | '\n' | '\r' | '\x0c' | ' '))
        .filter_map(|part| {
            // HTML's list-of-floating-point-numbers parser ignores leading garbage
            // and trailing nonnumeric text within each separator-delimited token.
            let start = part.find(|c: char| c.is_ascii_digit() || matches!(c, '-' | '+' | '.'))?;
            let part = &part[start..];
            let bytes = part.as_bytes();
            let mut end = 0;
            if bytes.get(end).is_some_and(|b| matches!(b, b'-' | b'+')) {
                end += 1;
            }
            let mut digits = 0;
            while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                end += 1;
                digits += 1;
            }
            if bytes.get(end) == Some(&b'.') {
                end += 1;
                while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                    end += 1;
                    digits += 1;
                }
            }
            if digits == 0 {
                return None;
            }
            if bytes.get(end).is_some_and(|b| matches!(b, b'e' | b'E')) {
                let exponent = end;
                end += 1;
                if bytes.get(end).is_some_and(|b| matches!(b, b'+' | b'-')) {
                    end += 1;
                }
                let first = end;
                while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                    end += 1;
                }
                if end == first {
                    end = exponent;
                }
            }
            part[..end]
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
        })
        .collect()
}
fn contains(shape: &str, coords: &str, x: f64, y: f64) -> bool {
    if shape.eq_ignore_ascii_case("default") {
        return true;
    }
    let n = numbers(coords);
    match shape.to_ascii_lowercase().as_str() {
        "circle" | "circ" => n.len() >= 3 && n[2] > 0. && (x - n[0]).hypot(y - n[1]) <= n[2],
        "poly" | "polygon" => {
            let points = n.chunks_exact(2).map(|p| (p[0], p[1])).collect::<Vec<_>>();
            if points.len() < 3 {
                return false;
            }
            let mut inside = false;
            let mut previous = points[points.len() - 1];
            for current in points {
                let (ax, ay) = previous;
                let (bx, by) = current;
                if (x - ax) * (by - ay) == (y - ay) * (bx - ax)
                    && x >= ax.min(bx)
                    && x <= ax.max(bx)
                    && y >= ay.min(by)
                    && y <= ay.max(by)
                {
                    return true;
                }
                if (ay > y) != (by > y) && x < (bx - ax) * (y - ay) / (by - ay) + ax {
                    inside = !inside;
                }
                previous = current;
            }
            inside
        }
        _ => {
            n.len() >= 4
                && x >= n[0].min(n[2])
                && x <= n[0].max(n[2])
                && y >= n[1].min(n[3])
                && y <= n[1].max(n[3])
        }
    }
}
fn region(
    dom: &oryn_dom::DomRuntime,
    image: NodeId,
    x: f64,
    y: f64,
) -> Result<Option<NodeId>, V8BindingError> {
    if !dom.is_element(image, "http://www.w3.org/1999/xhtml", "img")
        && !dom.is_element(image, "http://www.w3.org/1999/xhtml", "object")
    {
        return Ok(None);
    }
    let Some(reference) = dom.attribute_in_null_namespace(image, "usemap")? else {
        return Ok(None);
    };
    let Some((_, name)) = reference
        .split_once('#')
        .filter(|(_, name)| !name.is_empty())
    else {
        return Ok(None);
    };
    let mut root = image;
    while let Some(parent) = dom.arena().get(root)?.parent {
        root = parent;
    }
    let map = dom
        .arena()
        .descendants_inclusive(root)?
        .into_iter()
        .find(|node| {
            dom.is_element(*node, "http://www.w3.org/1999/xhtml", "map")
                && (dom
                    .attribute_in_null_namespace(*node, "name")
                    .ok()
                    .flatten()
                    == Some(name)
                    || dom.attribute_in_null_namespace(*node, "id").ok().flatten() == Some(name))
        });
    let Some(map) = map else { return Ok(None) };
    for area in dom.arena().descendants_inclusive(map)? {
        if !dom.is_element(area, "http://www.w3.org/1999/xhtml", "area") || dom.is_inert(area)? {
            continue;
        }
        if contains(
            dom.attribute_in_null_namespace(area, "shape")?
                .unwrap_or("rect"),
            dom.attribute_in_null_namespace(area, "coords")?
                .unwrap_or(""),
            x,
            y,
        ) {
            return Ok(Some(area));
        }
    }
    Ok(None)
}
pub(super) fn target(
    dom: &oryn_dom::DomRuntime,
    layout: &BindingLayout,
    image: NodeId,
    x: f64,
    y: f64,
) -> Result<NodeId, V8BindingError> {
    let key = node_key(image);
    let local = if let Some(projection) = layout.hit_projections.get(&key) {
        projection.viewport_to_local(x, y).and_then(|(x, y)| {
            let rect = projection.box_model.content_box;
            let x = x - rect.x;
            let y = y - rect.y;
            (x >= 0. && y >= 0. && x <= rect.width && y <= rect.height).then_some((x, y))
        })
    } else {
        match layout.geometry.get(&key) {
            Some(BindingGeometry::Available(rect)) => Some((x - rect.x, y - rect.y)),
            _ => None,
        }
    };
    let Some((x, y)) = local else {
        return Ok(image);
    };
    Ok(region(dom, image, x, y)?.unwrap_or(image))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pinned_shapes_and_permissive_coordinate_lists() {
        for value in [
            "2,2,10,10",
            "2;2;10;10",
            "2a2b20c20,2,10,10",
            "='2,2,10,10' ",
            ",,2;,;2,;,10 \t\r\n10;;",
            "2%,2%,1e1,1e1",
        ] {
            assert!(contains("rect", value, 5., 5.), "{value}");
        }
        for value in [
            "2\u{b}2\u{b}10\u{b}10",
            "2,2,10,",
            "2,,10,10",
            "2,2,10ls/spain/10/Canary",
        ] {
            assert!(!contains("rect", value, 5., 5.), "{value}");
        }
        assert!(contains("rect", "10,10,2,2", 5., 5.));
        assert!(contains("DEFAULT", "", 299., 299.));
        assert!(contains("CIRC", "20,40,10", 20., 40.));
        assert!(!contains("circle", "20,40,0", 20., 40.));
        assert!(!contains("circle", "20,40,-10", 20., 40.));
        assert!(contains("poly", "100,100,120,100,100,120,300", 105., 105.));
        assert!(!contains("poly", "100,100,120,100,100", 105., 105.));
    }
    #[test]
    fn map_reference_is_live_tree_order_exact_and_does_not_require_area_href() {
        let mut dom=html::parse("<img id=i usemap='http://example.test/#m'><map id=m><area id=a coords='0,0,20,20'></map><map name=m><area id=b shape=default></map>").into_runtime().unwrap();
        let image = dom.find_by_id("i").unwrap();
        let a = dom.find_by_id("a").unwrap();
        assert_eq!(region(&dom, image, 5., 5.).unwrap(), Some(a));
        assert_eq!(region(&dom, image, 30., 30.).unwrap(), None);
        let map = dom.find_by_id("m").unwrap();
        dom.set_attribute(map, "id", "other").unwrap();
        assert_eq!(region(&dom, image, 5., 5.).unwrap(), dom.find_by_id("b"));
        dom.set_attribute(image, "usemap", "#M").unwrap();
        assert_eq!(region(&dom, image, 5., 5.).unwrap(), None);
        dom.set_attribute(image, "usemap", "#%6d").unwrap();
        assert_eq!(region(&dom, image, 5., 5.).unwrap(), None);
    }
}
