# Site Console Sensor Navigation Design

## Goal

Make the Site Console navigation, URL, breadcrumb, and visible actions describe the same operator task. Viewing a sensor and configuring a sensor are separate tasks even when they use the same underlying data.

## Routes and responsibilities

- `/sensors` is the monitoring list for current sensor values.
- `/sensors/{signal_ref}` is the monitoring detail for one sensor. It shows the current value, reception state, graph, configured result summary, and source facts. It does not contain mutation forms.
- `/equipment` is the root of physical equipment management.
- `/equipment/devices/{device_ref}` is the device detail.
- `/equipment/devices/{device_ref}/sensors/{signal_ref}` is the canonical sensor settings page. It contains the existing basic, calibration, normal-value, and alarm editors.

The settings route returns `404 Not Found` when either resource does not exist or the sensor does not belong to the device in the URL. No compatibility route is added because IoTKit is still under development.

## Navigation

The monitoring detail keeps `センサー一覧` selected in the sidebar and uses the breadcrumb `センサー一覧 / {sensor}`.

The settings page keeps `機器管理` selected and uses the breadcrumb `機器管理 / {Edge} / {device} / {sensor}`. The device detail links directly to this settings route. An administrator sees `設定を変更` on the monitoring detail; viewers do not see a settings action.

After every successful or failed settings mutation, the browser returns to the canonical equipment settings URL and restores the edited tab or section.

## Monitoring detail

The monitoring detail is optimized for inspection rather than editing.

- The compact identity header shows sensor type, name, source, current value, and reception status.
- The graph remains the primary visual evidence.
- A concise settings summary shows the display profile, calibration, normal-value rules, and alarms without editable controls.
- Source and reception facts remain available through progressive disclosure.
- The administrator action `設定を変更` is visually distinct but secondary to the current value.

## Settings page

The settings page preserves the existing two-column editor: tabbed settings on the left and the live preview on the right. It changes only its navigation context and return URL.

The page heading and breadcrumb make the physical hierarchy clear. It does not duplicate a separate sensor implementation; monitoring and settings use the same view data, preview API, rule model, and mutation endpoints.

## Accessibility and responsive behavior

- Sidebar selection and `aria-current` always match the page responsibility.
- Both pages provide a breadcrumb labeled `現在位置`.
- Links use explicit labels: `センサーを確認` for monitoring and `センサー設定を開く` or `設定を変更` for configuration.
- Status is conveyed by text as well as color.
- Existing keyboard-operable tabs, focus restoration, graph text summary, and desktop responsive behavior remain intact.

## Verification

- Route tests cover the monitoring detail, settings detail, resource ownership validation, active navigation, breadcrumbs, and role-specific actions.
- Mutation tests verify canonical equipment return targets and reject malformed nested paths.
- Template tests verify that monitoring contains no mutation forms and settings contains all required editors.
- Frontend tests and the Site HTTP package test suite pass.
- Both pages are browser-reviewed at desktop and narrower desktop widths before sharing the running console.
