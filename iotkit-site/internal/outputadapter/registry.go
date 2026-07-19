package outputadapter

import (
	"fmt"
	"sort"
)

type Registry struct {
	adapters map[string]Adapter
}

func NewRegistry(adapters ...Adapter) (Registry, error) {
	registry := Registry{adapters: make(map[string]Adapter, len(adapters))}
	for _, adapter := range adapters {
		if adapter == nil {
			return Registry{}, fmt.Errorf(
				"%w: nil adapter",
				ErrInvalidDescriptor,
			)
		}
		descriptor := adapter.Descriptor()
		if err := descriptor.Validate(); err != nil {
			return Registry{}, err
		}
		if _, exists := registry.adapters[descriptor.ID]; exists {
			return Registry{}, fmt.Errorf(
				"%w: duplicate adapter ID %q",
				ErrInvalidDescriptor,
				descriptor.ID,
			)
		}
		registry.adapters[descriptor.ID] = adapter
	}
	return registry, nil
}

func BuiltInRegistry() (Registry, error) {
	return NewRegistry(
		GenericMQTTJSONAdapter{},
		YokaKitAdapter{},
	)
}

func (registry Registry) Resolve(id string) (Adapter, bool) {
	adapter, ok := registry.adapters[id]
	return adapter, ok
}

func (registry Registry) Descriptors() []Descriptor {
	descriptors := make([]Descriptor, 0, len(registry.adapters))
	for _, adapter := range registry.adapters {
		descriptors = append(descriptors, adapter.Descriptor())
	}
	sort.Slice(descriptors, func(left, right int) bool {
		return descriptors[left].ID < descriptors[right].ID
	})
	return descriptors
}
