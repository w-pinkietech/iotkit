package semantic

import "testing"

func TestMappingSpecValidate(t *testing.T) {
	valid := MappingSpec{
		EdgeNodeID:  "edge-node-01",
		SeriesKey:   "subject:contact_state:na:primary",
		Meaning:     MeaningProductionPulse,
		TriggerMode: TriggerActiveSample,
		ActiveValue: 1,
	}
	if err := valid.Validate(); err != nil {
		t.Fatal(err)
	}

	for _, bad := range []MappingSpec{
		{SeriesKey: valid.SeriesKey, Meaning: MeaningProductionPulse, TriggerMode: TriggerActiveSample, ActiveValue: 1},
		{EdgeNodeID: valid.EdgeNodeID, Meaning: MeaningProductionPulse, TriggerMode: TriggerActiveSample, ActiveValue: 1},
		{EdgeNodeID: "edge-node/node", SeriesKey: valid.SeriesKey, Meaning: MeaningProductionPulse, TriggerMode: TriggerActiveSample, ActiveValue: 1},
		{EdgeNodeID: "edge-node+node", SeriesKey: valid.SeriesKey, Meaning: MeaningProductionPulse, TriggerMode: TriggerActiveSample, ActiveValue: 1},
		{EdgeNodeID: "edge-node#node", SeriesKey: valid.SeriesKey, Meaning: MeaningProductionPulse, TriggerMode: TriggerActiveSample, ActiveValue: 1},
		{EdgeNodeID: valid.EdgeNodeID, SeriesKey: valid.SeriesKey, Meaning: "production", TriggerMode: TriggerActiveSample, ActiveValue: 1},
		{EdgeNodeID: valid.EdgeNodeID, SeriesKey: valid.SeriesKey, Meaning: MeaningProductionPulse, TriggerMode: "automatic", ActiveValue: 1},
		{EdgeNodeID: valid.EdgeNodeID, SeriesKey: valid.SeriesKey, Meaning: MeaningProductionPulse, TriggerMode: TriggerActiveEdge, ActiveValue: 2},
		{EdgeNodeID: valid.EdgeNodeID, SeriesKey: valid.SeriesKey, Meaning: MeaningProductionPulse},
	} {
		if bad.Validate() == nil {
			t.Fatalf("accepted invalid spec: %#v", bad)
		}
	}
}
