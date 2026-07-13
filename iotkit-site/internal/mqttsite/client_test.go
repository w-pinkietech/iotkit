package mqttsite

import "testing"

func TestRecordsTopicFilterUsesEdgeNodes(t *testing.T) {
	if recordsTopicFilter != "iotkit/v1/edge-nodes/+/records" {
		t.Fatalf("records topic filter = %q", recordsTopicFilter)
	}
}
