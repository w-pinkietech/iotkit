// Generated from edge/openapi/edge-console-v1.yaml. Do not edit.
export interface paths {
    "/api/v1/history": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Query a bounded page of durably stored sensor history */
        get: operations["listHistory"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/history/series": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Aggregate one raw sensor or processed measurement rule into bounded time buckets */
        get: operations["getHistorySeries"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/history.csv": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Export the same bounded history filter as UTF-8 CSV */
        get: operations["exportHistoryCSV"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/semantic-history.csv": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * Export persisted processed semantic observations as UTF-8 CSV
         * @description Exports the results stored when IoTKit Edge applied calibration and semantic rules. Historical observations are not recomputed with current rules.
         */
        get: operations["exportSemanticHistoryCSV"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/system/storage": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Read IoTKit Edge database, filesystem, and pending-work storage facts */
        get: operations["getStorageStatus"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/api/v1/system/diagnostics": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Read factual IoTKit Edge, Edge Node, sensor, output, and restore diagnostics */
        get: operations["getDiagnostics"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
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
         * @description Requires an authenticated IoTKit Console session, a matching X-CSRF-Token header, and a same-origin browser request. The schema documents the supported Console request; the server may continue to read older DefinitionSpec representations for stored-client compatibility.
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
        HistoryRecord: {
            signal_ref: string;
            series_key: string;
            edge_node_id: string;
            ledger_epoch: string;
            /** Format: int64 */
            pub_seq: number;
            /** Format: int64 */
            received_at: number;
            /** Format: int64 */
            observed_at: number;
            values: number[];
            value_type: string;
            unit: string;
            display_name: string;
            decimal_places: number;
            display_value_kind: string;
        };
        HistoryPage: {
            records: components["schemas"]["HistoryRecord"][];
            has_more: boolean;
            next_cursor?: string;
        };
        HistorySeriesPoint: {
            /** Format: int64 */
            bucket_start: number;
            minimum: number;
            average: number;
            maximum: number;
            /** Format: int64 */
            sample_count: number;
        };
        HistorySeries: {
            signal_ref: string;
            display_name: string;
            unit: string;
            value_type: string;
            /** Format: int64 */
            sample_count: number;
            /**
             * Format: int64
             * @description For signal_ref, exact IoTKit Edge receipt time of the latest raw record within the requested range. For rule_id, receipt time of the latest persisted processed observation regardless of the requested chart range. Null when the selected source has no value.
             */
            latest_received_at: number | null;
            /** @description For signal_ref, the first raw value from that latest in-range record. For rule_id, the latest persisted processed value regardless of the requested chart range. Null when the selected source has no value. */
            latest_value: number | boolean | string | null;
            points: components["schemas"]["HistorySeriesPoint"][];
        };
        StorageStatus: {
            /** @enum {string} */
            state: "healthy" | "warning" | "critical" | "unavailable";
            filesystem_available: boolean;
            /** Format: int64 */
            database_bytes: number;
            /** Format: int64 */
            reclaimable_bytes: number;
            /** Format: int64 */
            disk_total_bytes: number;
            /** Format: int64 */
            disk_available_bytes: number;
            disk_used_percent: number;
            warning_percent: number;
            /** Format: int64 */
            raw_record_count: number;
            /** Format: int64 */
            semantic_observation_count: number;
            /** Format: int64 */
            pending_output_count: number;
            /** Format: int64 */
            projection_failure_count: number;
            last_backup_id?: string;
            /** Format: int64 */
            last_backup_at?: number;
            /** Format: int64 */
            last_backup_raw_record_count?: number;
            /** Format: int64 */
            backup_protected_raw_count: number;
            /** Format: int64 */
            unprotected_raw_count: number;
            automatic_raw_purge_enabled: boolean;
        };
        DiagnosticIssue: {
            code: string;
            /** @enum {string} */
            severity: "info" | "warning" | "critical";
            component: string;
            resource_ref?: string;
            summary: string;
            detail: string;
            /** Format: int64 */
            observed_at?: number;
        };
        DiagnosticReport: {
            /** Format: int64 */
            generated_at: number;
            /** @enum {string} */
            state: "healthy" | "attention" | "critical";
            issues: components["schemas"]["DiagnosticIssue"][];
            truncated: boolean;
            limitations: string[];
        };
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
        /** @description Inclusive IoTKit Edge receipt time in Unix milliseconds. */
        HistoryFrom: number;
        /** @description Exclusive IoTKit Edge receipt time in Unix milliseconds; ranges are limited to 31 days. */
        HistoryTo: number;
        /** @description Raw sensor selection when rule_id is absent. For /history/series, both buckets and latest_* are constrained to the requested range. */
        HistorySignalRef: string;
        HistorySignalRefRequired: string;
        HistoryEdgeNodeID: string;
        /** @description Value of the same-origin iotkit_edge_csrf cookie. */
        CSRFToken: string;
    };
    requestBodies: never;
    headers: never;
    pathItems: never;
}
export type $defs = Record<string, never>;
export interface operations {
    listHistory: {
        parameters: {
            query: {
                /** @description Inclusive IoTKit Edge receipt time in Unix milliseconds. */
                from: components["parameters"]["HistoryFrom"];
                /** @description Exclusive IoTKit Edge receipt time in Unix milliseconds; ranges are limited to 31 days. */
                to: components["parameters"]["HistoryTo"];
                /** @description Raw sensor selection when rule_id is absent. For /history/series, both buckets and latest_* are constrained to the requested range. */
                signal_ref?: components["parameters"]["HistorySignalRef"];
                edge_node_id?: components["parameters"]["HistoryEdgeNodeID"];
                limit?: number;
                cursor?: string;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description A stable newest-first history page */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["HistoryPage"];
                };
            };
            default: components["responses"]["RequestError"];
        };
    };
    getHistorySeries: {
        parameters: {
            query: {
                /** @description Inclusive IoTKit Edge receipt time in Unix milliseconds. */
                from: components["parameters"]["HistoryFrom"];
                /** @description Exclusive IoTKit Edge receipt time in Unix milliseconds; ranges are limited to 31 days. */
                to: components["parameters"]["HistoryTo"];
                /** @description Raw sensor selection when rule_id is absent. For /history/series, both buckets and latest_* are constrained to the requested range. */
                signal_ref?: components["parameters"]["HistorySignalRef"];
                /** @description Active semantic rule to read instead of raw sensor values. Buckets honor the requested range; latest_* reports the latest persisted processed observation regardless of that range. */
                rule_id?: string;
                bucket_ms: number;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description At most 1000 chronological aggregate points */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["HistorySeries"];
                };
            };
            default: components["responses"]["RequestError"];
        };
    };
    exportHistoryCSV: {
        parameters: {
            query: {
                /** @description Inclusive IoTKit Edge receipt time in Unix milliseconds. */
                from: components["parameters"]["HistoryFrom"];
                /** @description Exclusive IoTKit Edge receipt time in Unix milliseconds; ranges are limited to 31 days. */
                to: components["parameters"]["HistoryTo"];
                /** @description Raw sensor selection when rule_id is absent. For /history/series, both buckets and latest_* are constrained to the requested range. */
                signal_ref?: components["parameters"]["HistorySignalRef"];
                edge_node_id?: components["parameters"]["HistoryEdgeNodeID"];
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description CSV with a UTF-8 BOM for spreadsheet interoperability */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "text/csv": string;
                };
            };
            /** @description More than 100000 rows matched; narrow the filter */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorResponse"];
                };
            };
            default: components["responses"]["RequestError"];
        };
    };
    exportSemanticHistoryCSV: {
        parameters: {
            query: {
                /** @description Inclusive IoTKit Edge receipt time in Unix milliseconds. */
                from: components["parameters"]["HistoryFrom"];
                /** @description Exclusive IoTKit Edge receipt time in Unix milliseconds; ranges are limited to 31 days. */
                to: components["parameters"]["HistoryTo"];
                /** @description Raw sensor selection when rule_id is absent. For /history/series, both buckets and latest_* are constrained to the requested range. */
                signal_ref?: components["parameters"]["HistorySignalRef"];
                edge_node_id?: components["parameters"]["HistoryEdgeNodeID"];
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Processed observation CSV with a UTF-8 BOM */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "text/csv": string;
                };
            };
            /** @description More than 100000 observations matched; narrow the filter */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorResponse"];
                };
            };
            default: components["responses"]["RequestError"];
        };
    };
    getStorageStatus: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Current storage facts observed by IoTKit Edge */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["StorageStatus"];
                };
            };
            default: components["responses"]["RequestError"];
        };
    };
    getDiagnostics: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Bounded diagnostic issues and explicit observability limitations */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["DiagnosticReport"];
                };
            };
            default: components["responses"]["RequestError"];
        };
    };
    createMappingPreview: {
        parameters: {
            query?: never;
            header: {
                /** @description Value of the same-origin iotkit_edge_csrf cookie. */
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
