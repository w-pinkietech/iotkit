use super::{
    DefinitionSpec, Evaluation, EvaluationState, SemanticError, SemanticKind, evaluate_at,
};

#[derive(Debug, Clone, Copy)]
pub struct PreviewInput {
    pub received_at: i64,
    pub observed_at: Option<i64>,
    pub value: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct PreviewPoint {
    pub received_at: i64,
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
    pub points: Vec<PreviewPoint>,
    pub test_result: Option<Evaluation>,
}

pub fn build_preview(
    spec: DefinitionSpec,
    inputs: &[PreviewInput],
    max_points: usize,
    test_value: Option<f64>,
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
        let (result, next) = evaluate_at(
            spec,
            state,
            input.value,
            input.observed_at.unwrap_or(input.received_at),
        )?;
        state = next;
        let active = (spec.kind != SemanticKind::Numeric).then_some(state.active);
        points.push(PreviewPoint {
            received_at: input.received_at,
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
    if points.len() > max_points {
        points = summarize(&points, max_points);
    }
    let test_result = test_value
        .map(|value| evaluate_at(spec, EvaluationState::default(), value, 0))
        .transpose()?
        .map(|pair| pair.0);
    Ok(Preview {
        input_count: inputs.len(),
        points,
        test_result,
    })
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
