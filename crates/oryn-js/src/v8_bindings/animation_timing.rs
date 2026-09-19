//! Native Web Animations timing model. Times are document-timeline milliseconds.
//! The binding and renderer use this result rather than independent clocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fill {
    None,
    Forwards,
    Backwards,
    Both,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Direction {
    Normal,
    Reverse,
    Alternate,
    AlternateReverse,
}
#[derive(Clone, Debug)]
pub(super) struct Timing {
    pub delay: f64,
    pub end_delay: f64,
    pub duration: Option<f64>,
    pub iterations: f64,
    pub iteration_start: f64,
    pub fill: Fill,
    pub direction: Direction,
    pub easing: Easing,
}
impl Default for Timing {
    fn default() -> Self {
        Self {
            delay: 0.,
            end_delay: 0.,
            duration: None,
            iterations: 1.,
            iteration_start: 0.,
            fill: Fill::None,
            direction: Direction::Normal,
            easing: Easing::Linear,
        }
    }
}
#[derive(Clone, Debug)]
pub(super) enum Easing {
    Linear,
    Cubic(f64, f64, f64, f64),
    Steps(u32, StepPosition),
    Piecewise(Vec<(f64, f64)>),
}
#[derive(Clone, Copy, Debug)]
pub(super) enum StepPosition {
    Start,
    End,
    None,
    Both,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Before,
    Active,
    After,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Sample {
    pub phase: Phase,
    pub active_duration: f64,
    pub end_time: f64,
    pub active_time: Option<f64>,
    pub iteration: Option<f64>,
    pub progress: Option<f64>,
}
impl Timing {
    pub fn active_duration(&self) -> f64 {
        let duration = self.duration.unwrap_or(0.);
        if duration == 0. || self.iterations == 0. {
            0.
        } else {
            duration * self.iterations
        }
    }
    pub fn end_time(&self) -> f64 {
        (self.delay + self.active_duration() + self.end_delay).max(0.)
    }
    pub fn sample(&self, time: Option<f64>, rate: f64) -> Sample {
        let active_duration = self.active_duration();
        let end_time = self.end_time();
        let Some(time) = time else {
            return Sample {
                phase: Phase::Before,
                active_duration,
                end_time,
                active_time: None,
                iteration: None,
                progress: None,
            };
        };
        let before = self.delay.min(end_time).max(0.);
        let after = (self.delay + active_duration).min(end_time).max(0.);
        let phase = if time < before || (time == before && rate < 0.) {
            Phase::Before
        } else if time > after || (time == after && rate >= 0.) {
            Phase::After
        } else {
            Phase::Active
        };
        let active_time = match phase {
            Phase::Before if matches!(self.fill, Fill::Backwards | Fill::Both) => {
                Some((time - self.delay).max(0.))
            }
            Phase::After if matches!(self.fill, Fill::Forwards | Fill::Both) => {
                Some((time - self.delay).max(0.).min(active_duration))
            }
            Phase::Active => Some(time - self.delay),
            _ => None,
        };
        let Some(active) = active_time else {
            return Sample {
                phase,
                active_duration,
                end_time,
                active_time,
                iteration: None,
                progress: None,
            };
        };
        let duration = self.duration.unwrap_or(0.);
        let overall = if duration == 0. {
            self.iteration_start
                + if phase == Phase::Before {
                    0.
                } else {
                    self.iterations
                }
        } else {
            self.iteration_start + active / duration
        };
        let mut simple = if overall.is_infinite() {
            self.iteration_start.fract()
        } else {
            overall.fract()
        };
        if simple == 0.
            && self.iterations != 0.
            && (phase == Phase::After || active == active_duration)
        {
            simple = 1.;
        }
        let iteration = if phase == Phase::After && self.iterations.is_infinite() {
            f64::INFINITY
        } else if simple == 1. {
            overall.floor() - 1.
        } else {
            overall.floor()
        };
        let reverse = match self.direction {
            Direction::Normal => false,
            Direction::Reverse => true,
            Direction::Alternate => iteration % 2. != 0.,
            Direction::AlternateReverse => iteration % 2. == 0.,
        };
        let directed = if reverse { 1. - simple } else { simple };
        let before_flag = phase == Phase::Before && !reverse || phase == Phase::After && reverse;
        let progress = self.easing.sample(directed, before_flag);
        Sample {
            phase,
            active_duration,
            end_time,
            active_time,
            iteration: Some(iteration),
            progress: Some(progress),
        }
    }
}
impl Easing {
    pub fn parse(input: &str) -> Option<Self> {
        let text = input.trim().to_ascii_lowercase();
        match text.as_str() {
            "linear" => return Some(Self::Linear),
            "ease" => return Some(Self::Cubic(0.25, 0.1, 0.25, 1.)),
            "ease-in" => return Some(Self::Cubic(0.42, 0., 1., 1.)),
            "ease-out" => return Some(Self::Cubic(0., 0., 0.58, 1.)),
            "ease-in-out" => return Some(Self::Cubic(0.42, 0., 0.58, 1.)),
            "step-start" => return Some(Self::Steps(1, StepPosition::Start)),
            "step-end" => return Some(Self::Steps(1, StepPosition::End)),
            _ => {}
        }
        if let Some(args) = text
            .strip_prefix("cubic-bezier(")
            .and_then(|text| text.strip_suffix(')'))
        {
            let values = args
                .split(',')
                .map(|value| value.trim().parse::<f64>().ok())
                .collect::<Option<Vec<_>>>()?;
            if values.len() != 4
                || values.iter().any(|value| !value.is_finite())
                || !(0. ..=1.).contains(&values[0])
                || !(0. ..=1.).contains(&values[2])
            {
                return None;
            }
            return Some(Self::Cubic(values[0], values[1], values[2], values[3]));
        }
        if let Some(args) = text
            .strip_prefix("linear(")
            .and_then(|text| text.strip_suffix(')'))
        {
            let mut stops = Vec::new();
            for part in args.split(',') {
                let mut parts = part.split_whitespace();
                let output = parts.next()?.parse::<f64>().ok()?;
                if !output.is_finite() {
                    return None;
                }
                let positions = parts
                    .map(|value| {
                        value
                            .strip_suffix('%')?
                            .parse::<f64>()
                            .ok()
                            .filter(|value| value.is_finite())
                            .map(|value| value / 100.)
                    })
                    .collect::<Option<Vec<_>>>()?;
                if positions.len() > 2 {
                    return None;
                }
                if positions.is_empty() {
                    stops.push((None, output));
                } else {
                    for position in positions {
                        stops.push((Some(position), output));
                    }
                }
            }
            if stops.len() < 2 {
                return None;
            }
            if stops[0].0.is_none() {
                stops[0].0 = Some(0.);
            }
            let last = stops.len() - 1;
            if stops[last].0.is_none() {
                stops[last].0 = Some(1.);
            }
            let mut previous = f64::NEG_INFINITY;
            for (position, _) in &mut stops {
                if let Some(value) = position {
                    *value = value.max(previous);
                    previous = *value;
                }
            }
            let mut start = 0;
            while start < last {
                let end = ((start + 1)..=last).find(|index| stops[*index].0.is_some())?;
                let left = stops[start].0?;
                let right = stops[end].0?;
                for index in start + 1..end {
                    stops[index].0 =
                        Some(left + (right - left) * (index - start) as f64 / (end - start) as f64);
                }
                start = end;
            }
            return Some(Self::Piecewise(
                stops
                    .into_iter()
                    .map(|(position, output)| (position.unwrap(), output))
                    .collect(),
            ));
        }
        if let Some(args) = text
            .strip_prefix("steps(")
            .and_then(|text| text.strip_suffix(')'))
        {
            let mut parts = args.split(',');
            let count = parts.next()?.trim().parse::<u32>().ok()?;
            let position = match parts.next().unwrap_or("end").trim() {
                "start" | "jump-start" => StepPosition::Start,
                "end" | "jump-end" => StepPosition::End,
                "jump-none" => StepPosition::None,
                "jump-both" => StepPosition::Both,
                _ => return None,
            };
            if parts.next().is_some()
                || count == 0
                || count == 1 && matches!(position, StepPosition::None)
            {
                return None;
            }
            return Some(Self::Steps(count, position));
        }
        None
    }
    pub fn sample(&self, progress: f64, before: bool) -> f64 {
        if let Self::Piecewise(stops) = self {
            if progress < stops[0].0 && stops[0].0 == stops[1].0 {
                return stops[0].1;
            }
            let last = stops.len() - 1;
            if progress > stops[last].0 && stops[last].0 == stops[last - 1].0 {
                return stops[last].1;
            }
            let end = stops
                .iter()
                .position(|(position, _)| *position > progress || before && *position == progress)
                .unwrap_or(stops.len() - 1)
                .max(1);
            let (left, right) = (stops[end - 1], stops[end]);
            if left.0 == right.0 {
                return if before { left.1 } else { right.1 };
            }
            return left.1 + (right.1 - left.1) * (progress - left.0) / (right.0 - left.0);
        }
        match *self {
            Self::Piecewise(_) => unreachable!(),
            Self::Linear => progress,
            Self::Steps(count, position) => {
                let count = count as f64;
                let scaled = progress * count;
                let mut step = scaled.floor();
                if matches!(position, StepPosition::Start | StepPosition::Both) {
                    step += 1.;
                }
                if before && scaled.fract() == 0. {
                    step -= 1.;
                }
                let jumps = count
                    + match position {
                        StepPosition::None => -1.,
                        StepPosition::Both => 1.,
                        _ => 0.,
                    };
                if progress >= 0. && step < 0. {
                    step = 0.;
                }
                if progress <= 1. && step > jumps {
                    step = jumps;
                }
                step / jumps
            }
            Self::Cubic(x1, y1, x2, y2) => {
                if progress < 0. {
                    let slope = if x1 > 0. {
                        y1 / x1
                    } else if x2 > 0. {
                        y2 / x2
                    } else {
                        0.
                    };
                    return progress * slope;
                }
                if progress > 1. {
                    let slope = if x2 < 1. {
                        (1. - y2) / (1. - x2)
                    } else if x1 < 1. {
                        (1. - y1) / (1. - x1)
                    } else {
                        0.
                    };
                    return 1. + (progress - 1.) * slope;
                }
                let bezier = |t: f64, a: f64, b: f64| {
                    3. * (1. - t) * (1. - t) * t * a + 3. * (1. - t) * t * t * b + t * t * t
                };
                let (mut low, mut high) = (0., 1.);
                for _ in 0..60 {
                    let middle = (low + high) / 2.;
                    if bezier(middle, x1, x2) < progress {
                        low = middle
                    } else {
                        high = middle
                    }
                }
                bezier((low + high) / 2., y1, y2)
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timing_tracks_delay_fill_iteration_direction_and_negative_end_delay() {
        let mut timing = Timing {
            duration: Some(100.),
            delay: 20.,
            iterations: 2.,
            fill: Fill::Both,
            direction: Direction::Alternate,
            ..Default::default()
        };
        assert_eq!(timing.sample(Some(10.), 1.).progress, Some(0.));
        assert_eq!(timing.sample(Some(70.), 1.).progress, Some(0.5));
        assert_eq!(timing.sample(Some(120.), 1.).iteration, Some(1.));
        assert_eq!(timing.sample(Some(220.), 1.).progress, Some(0.));
        timing.end_delay = -50.;
        assert_eq!(timing.sample(Some(170.), 1.).progress, Some(0.5));
        assert_eq!(timing.end_time(), 170.);
    }
    #[test]
    fn easing_retains_interpolation_extrapolation_and_step_boundaries() {
        for progress in [-1.5, -0.3, 0., 0.5, 1., 1.5] {
            let b = (8. * progress - 1.) / 6.;
            let easing = Easing::parse(&format!("cubic-bezier(0,{b},1,{b})")).unwrap();
            assert!((easing.sample(0.5, false) - progress).abs() < 1e-12);
        }
        assert_eq!(Easing::parse("steps(1,end)").unwrap().sample(1., true), 0.);
        assert_eq!(
            Easing::parse("steps(1,start)").unwrap().sample(0., false),
            1.
        );
        assert!(Easing::parse("steps(1,jump-none)").is_none());
    }
    #[test]
    fn zero_and_infinite_duration_do_not_produce_nan_progress() {
        let mut timing = Timing {
            fill: Fill::Both,
            iterations: f64::INFINITY,
            ..Default::default()
        };
        assert_eq!(timing.active_duration(), 0.);
        assert_eq!(timing.sample(Some(0.), 1.).progress, Some(1.));
        timing.duration = Some(f64::INFINITY);
        assert_eq!(timing.sample(Some(50.), 1.).progress, Some(0.));
        assert!(timing.sample(None, 1.).progress.is_none());
    }
    #[test]
    fn piecewise_easing_distributes_stops_and_negative_delay_keeps_before_phase() {
        let easing = Easing::parse("linear(0, 0.25 25% 50%, 1)").unwrap();
        assert_eq!(easing.sample(0.375, false), 0.25);
        assert_eq!(easing.sample(0.75, false), 0.625);
        let timing = Timing {
            duration: Some(100.),
            delay: -50.,
            fill: Fill::Both,
            ..Default::default()
        };
        assert_eq!(timing.sample(Some(-1.), 1.).phase, Phase::Before);
        assert_eq!(timing.sample(Some(0.), 1.).progress, Some(0.5));
    }
}
