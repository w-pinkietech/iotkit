// Generated from iotkit-site/openapi/site-console-v1.yaml. Do not edit.
export interface paths {
    "/api/v1/mapping-previews": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get?: never;
        put?: never;
        /**
         * Preview future-only semantic settings without writing them
         * @description Requires an authenticated Site Console session, a matching X-CSRF-Token header, and a same-origin browser request. The schema documents the supported Console request; the server may continue to read older DefinitionSpec representations for stored-client compatibility.
         */
        post: operations["createMappingPreview"];
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
}
export type webhooks = Record<string, never>;
export interface components {
    schemas: {
        /** @enum {string} */
        SemanticKind: "numeric" | "boolean" | "cumulative_counter" | "alarm";
        /** @enum {string} */
        DetectorMode: "" | "boolean_high_active" | "boolean_low_active" | "high_active" | "low_active";
        /** @enum {string} */
        TriggerMode: "" | "on_transition" | "on_notification";
        Detector: {
            mode?: components["schemas"]["DetectorMode"];
            rise_threshold?: number;
            fall_threshold?: number;
            /** Format: int64 */
            rise_debounce_ms?: number;
            /** Format: int64 */
            fall_debounce_ms?: number;
        };
        LegacyCondition: {
            mode?: string;
            bool_value?: boolean;
            threshold?: number;
            hysteresis?: number;
        };
        DefinitionSpec: {
            kind: components["schemas"]["SemanticKind"];
            scale: number;
            offset?: number;
            detector?: components["schemas"]["Detector"];
            condition?: components["schemas"]["LegacyCondition"];
            trigger?: components["schemas"]["TriggerMode"];
        };
        RuleSpec: {
            kind: components["schemas"]["SemanticKind"];
            detector?: components["schemas"]["Detector"];
            trigger?: components["schemas"]["TriggerMode"];
        };
        CalibrationDraft: {
            signal_ref?: string;
            /** Format: int64 */
            revision?: number;
            scale: number;
            offset?: number;
            /** Format: int64 */
            created_at?: number;
        };
        SemanticRulePreviewDraft: {
            rule_id: string;
            display_name: string;
            spec: components["schemas"]["RuleSpec"];
        };
        MappingPreviewRequest: {
            signal_ref: string;
            spec?: components["schemas"]["DefinitionSpec"];
            test_value?: number;
            calibration?: components["schemas"]["CalibrationDraft"];
            rules?: components["schemas"]["SemanticRulePreviewDraft"][];
        };
        PreviewPoint: {
            /** Format: int64 */
            received_at: number;
            input: number;
            input_min: number;
            input_max: number;
            calibrated: number;
            calibrated_min: number;
            calibrated_max: number;
            active?: boolean;
            /** Format: int64 */
            counter?: number;
            sample_count: number;
            active_samples?: number;
            transitions?: number;
            /** Format: int64 */
            increment?: number;
        };
        PreviewResult: {
            emitted: boolean;
            number?: number;
            boolean?: boolean;
            /** Format: int64 */
            integer?: number;
            calibrated: number;
        };
        PreviewBody: {
            kind: components["schemas"]["SemanticKind"];
            input_count: number;
            plot_count: number;
            points: components["schemas"]["PreviewPoint"][] | null;
            test_result?: components["schemas"]["PreviewResult"];
            /** Format: int64 */
            window_start?: number;
            /** Format: int64 */
            window_end?: number;
            truncated_by?: string;
            rise_threshold?: number;
            fall_threshold?: number;
            error?: string;
        };
        MappingPreview: components["schemas"]["PreviewBody"];
        SemanticRulePreview: components["schemas"]["PreviewBody"] & {
            rule_id: string;
            display_name: string;
        };
        MultipleRuleMappingPreview: {
            calibration: components["schemas"]["Calibration"];
            rules: components["schemas"]["SemanticRulePreview"][];
            /** Format: int64 */
            window_start?: number;
            /** Format: int64 */
            window_end?: number;
            truncated_by?: string;
        };
        Calibration: {
            signal_ref: string;
            /** Format: int64 */
            revision: number;
            scale: number;
            offset: number;
            /** Format: int64 */
            created_at: number;
        };
        ErrorDetail: {
            code: string;
            message: string;
            field: string | null;
            request_id: string;
        };
        ErrorResponse: {
            error: components["schemas"]["ErrorDetail"];
        };
    };
    responses: {
        /** @description The request could not be completed */
        RequestError: {
            headers: {
                [name: string]: unknown;
            };
            content: {
                "application/json": components["schemas"]["ErrorResponse"];
            };
        };
    };
    parameters: {
        /** @description Value of the same-origin iotkit_site_csrf cookie. */
        CSRFToken: string;
    };
    requestBodies: never;
    headers: never;
    pathItems: never;
}
export type $defs = Record<string, never>;
export interface operations {
    createMappingPreview: {
        parameters: {
            query?: never;
            header: {
                /** @description Value of the same-origin iotkit_site_csrf cookie. */
                "X-CSRF-Token": components["parameters"]["CSRFToken"];
            };
            path?: never;
            cookie?: never;
        };
        requestBody: {
            content: {
                "application/json": components["schemas"]["MappingPreviewRequest"];
            };
        };
        responses: {
            /** @description Preview built from the bounded recent input window */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["MappingPreview"] | components["schemas"]["MultipleRuleMappingPreview"];
                };
            };
            400: components["responses"]["RequestError"];
            401: components["responses"]["RequestError"];
            403: components["responses"]["RequestError"];
            404: components["responses"]["RequestError"];
            default: components["responses"]["RequestError"];
        };
    };
}
