# Site Console Sensor Editor Simplification

## Goal

Reduce visual noise in the sensor editor without removing the settings operators need. An operator must be able to identify the sensor, compare incoming data with the configured result, and save one category of settings without scanning duplicate summaries or scrolling through unrelated controls.

## Scope

This change only reorganizes the Site Console sensor detail page. It does not change the Site API, database schema, MQTT contracts, rule semantics, permissions, or authentication.

## Layout

The page uses an editing-focused two-column layout.

- The compact page header contains the back link, sensor name, latest value, and reception state.
- The persistent three-stage data-flow panel is removed from this page.
- The left column contains the Basic, Normal value, and Alarm tabs and the active tab's editable form.
- The right column contains the live value and graph. Secondary preview controls use progressive disclosure.
- At narrower desktop widths, the preview follows the editor in one column.

## Information hierarchy

Each piece of information appears once.

- Remove the rule summary card shown above the same rule's edit form.
- Keep the rule selector only when a tab contains multiple rules.
- Use whitespace and separators for grouping; reserve bordered cards for the editor and preview columns.
- Use orange only for the primary save action and current navigation. Use teal for live data and links.
- Keep advanced debounce and destructive actions collapsed by default.

## Interaction

- Switching tabs changes only the left editor. The live preview remains visible.
- Returning after save restores the active tab and focused field.
- The save action stays at the end of the active form and remains visible within the initial desktop viewport when the common fields fit.
- Keyboard tab selection, switch semantics, inline validation, and the accessible chart summary remain unchanged.

## Verification

- Template tests assert that the removed flow and duplicate rule summary are absent from the sensor editor.
- Frontend tests continue to cover tab switching, focus restoration, preview control semantics, accessible chart summaries, and inline errors.
- The page is visually reviewed at 1440×1024 and a narrower desktop width.
- The complete Site HTTP and frontend test suites pass.
