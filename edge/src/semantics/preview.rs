use super::{
    DefinitionSpec, Evaluation, EvaluationState, SemanticError, SemanticKind, evaluate_at,
};

#[derive(Debug, Clone, Copy)]
pub struct PreviewInput {
    pub received_at: i64,
    pub observed_at: Option<i64>,
    pub value: f64,
}

impl PreviewInput {
    #[must_use]
    pub fn plot_at(self) -> i64 {
        self.observed_at
            .filter(|value| *value > 0)
            .unwrap_or(self.received_at)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PreviewPoint {
    pub received_at: i64,
    pub plot_at: i64,
    pub input: f64,
    pub input_min: f64,
    pub input_max: f64,
    pub calibrated: f64,
    pub calibrated_min: f64,
    pub calibrated_max: f64,
    pub active: Option<bool>,
    pub counter: Option<i64>,
    pub sample_count: usize,
    pub active_samples: usize,
    pub transitions: usize,
    pub increment: i64,
}

pub struct Preview {
    pub input_count: usize,
    pub plot_count: usize,
    pub points: Vec<PreviewPoint>,
    pub latest_point: Option<PreviewPoint>,
    pub test_result: Option<Evaluation>,
}

pub fn build_preview(
    spec: DefinitionSpec,
    inputs: &[PreviewInput],
    max_points: usize,
    test_value: Option<f64>,
) -> Result<Preview, SemanticError> {
    build_preview_window(spec, inputs, max_points, test_value, None)
}

/// Evaluate every bounded input so stateful rules retain their history, while
/// optionally limiting only the points returned for plotting.
pub fn build_preview_window(
    spec: DefinitionSpec,
    inputs: &[PreviewInput],
    max_points: usize,
    test_value: Option<f64>,
    plot_start: Option<i64>,
) -> Result<Preview, SemanticError> {
    if max_points == 0 {
        return Err(SemanticError::Invalid(
            "preview point limit must be positive".into(),
        ));
    }
    let mut state = EvaluationState::default();
    let mut points = Vec::with_capacity(inputs.len());
    for input in inputs {
        let previous = state;
        let plot_at = input.plot_at();
        let (result, next) = evaluate_at(spec, state, input.value, plot_at)?;
        state = next;
        let active = (spec.kind != SemanticKind::Numeric).then_some(state.active);
        points.push(PreviewPoint {
            received_at: input.received_at,
            plot_at,
            input: input.value,
            input_min: input.value,
            input_max: input.value,
            calibrated: result.calibrated,
            calibrated_min: result.calibrated,
            calibrated_max: result.calibrated,
            active,
            counter: (spec.kind == SemanticKind::CumulativeCounter).then_some(state.counter),
            sample_count: 1,
            active_samples: usize::from(active == Some(true)),
            transitions: usize::from(previous.initialized && previous.active != state.active),
            increment: state.counter - previous.counter,
        });
    }
    let latest_point = points.last().copied();
    if let Some(plot_start) = plot_start {
        points = bucket_by_second(&points, plot_start);
        if points.len() > max_points {
            points = summarize(&points, max_points);
        }
    } else if points.len() > max_points {
        points = summarize(&points, max_points);
    }
    let test_result = test_value
        .map(|value| evaluate_at(spec, EvaluationState::default(), value, 0))
        .transpose()?
        .map(|pair| pair.0);
    Ok(Preview {
        input_count: inputs.len(),
        plot_count: points.len(),
        points,
        latest_point,
        test_result,
    })
}

/// Collapse the already-evaluated recent window into time-based one-second
/// buckets. The plotted value follows the live chart's average/minimum/maximum
/// meaning; the receipt-order latest point is returned separately.
fn bucket_by_second(points: &[PreviewPoint], plot_start: i64) -> Vec<PreviewPoint> {
    let mut plotted: Vec<_> = points
        .iter()
        .filter(|point| point.plot_at >= plot_start)
        .copied()
        .collect();
    plotted.sort_by_key(|point| point.plot_at);
    let mut buckets = Vec::new();
    for point in plotted {
        let elapsed = point.plot_at.saturating_sub(plot_start);
        let bucket_start = plot_start.saturating_add(elapsed.div_euclid(1_000) * 1_000);
        if buckets
            .last()
            .is_some_and(|bucket: &PreviewBucket| bucket.plot_at == bucket_start)
        {
            buckets.last_mut().expect("bucket exists").add(point);
        } else {
            buckets.push(PreviewBucket::new(bucket_start, point));
        }
    }
    buckets.into_iter().map(PreviewBucket::finish).collect()
}

struct PreviewBucket {
    plot_at: i64,
    point: PreviewPoint,
    input_sum: f64,
    calibrated_sum: f64,
    samples: usize,
}

impl PreviewBucket {
    fn new(plot_at: i64, point: PreviewPoint) -> Self {
        let samples = point.sample_count.max(1);
        Self {
            plot_at,
            point,
            input_sum: point.input * samples as f64,
            calibrated_sum: point.calibrated * samples as f64,
            samples,
        }
    }

    fn add(&mut self, point: PreviewPoint) {
        let samples = point.sample_count.max(1);
        self.input_sum += point.input * samples as f64;
        self.calibrated_sum += point.calibrated * samples as f64;
        self.samples += samples;
        self.point.received_at = self.point.received_at.max(point.received_at);
        self.point.input_min = self.point.input_min.min(point.input_min);
        self.point.input_max = self.point.input_max.max(point.input_max);
        self.point.calibrated_min = self.point.calibrated_min.min(point.calibrated_min);
        self.point.calibrated_max = self.point.calibrated_max.max(point.calibrated_max);
        self.point.active = point.active;
        self.point.counter = point.counter;
        self.point.active_samples += point.active_samples;
        self.point.transitions += point.transitions;
        self.point.increment += point.increment;
    }

    fn finish(mut self) -> PreviewPoint {
        self.point.plot_at = self.plot_at;
        self.point.input = self.input_sum / self.samples as f64;
        self.point.calibrated = self.calibrated_sum / self.samples as f64;
        self.point.sample_count = self.samples;
        self.point
    }
}

fn summarize(points: &[PreviewPoint], max_points: usize) -> Vec<PreviewPoint> {
    (0..max_points)
        .filter_map(|bucket| {
            let start = bucket * points.len() / max_points;
            let end = (bucket + 1) * points.len() / max_points;
            (start < end).then(|| {
                let mut point = points[end - 1];
                point.input_min = points[start..end]
                    .iter()
                    .map(|value| value.input_min)
                    .fold(f64::INFINITY, f64::min);
                point.input_max = points[start..end]
                    .iter()
                    .map(|value| value.input_max)
                    .fold(f64::NEG_INFINITY, f64::max);
                point.calibrated_min = points[start..end]
                    .iter()
                    .map(|value| value.calibrated_min)
                    .fold(f64::INFINITY, f64::min);
                point.calibrated_max = points[start..end]
                    .iter()
                    .map(|value| value.calibrated_max)
                    .fold(f64::NEG_INFINITY, f64::max);
                point.sample_count = points[start..end]
                    .iter()
                    .map(|value| value.sample_count)
                    .sum();
                point.active_samples = points[start..end]
                    .iter()
                    .map(|value| value.active_samples)
                    .sum();
                point.transitions = points[start..end]
                    .iter()
                    .map(|value| value.transitions)
                    .sum();
                point.increment = points[start..end].iter().map(|value| value.increment).sum();
                point
            })
        })
        .collect()
}
