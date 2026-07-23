package store

import "errors"

type Profile string

const (
	ProfileEmbedded Profile = "embedded"
	ProfilePostgres Profile = "postgres"
)

type OpenOptions struct {
	Profile     Profile
	SQLitePath  string
	PostgresDSN string
	EdgeID      string
}

func (options OpenOptions) normalized() (OpenOptions, error) {
	if options.Profile == "" {
		options.Profile = ProfileEmbedded
	}
	switch options.Profile {
	case ProfileEmbedded:
		if options.SQLitePath == "" {
			return OpenOptions{}, errors.New("SQLite path is required for embedded storage")
		}
		if options.PostgresDSN != "" {
			return OpenOptions{}, errors.New("PostgreSQL configuration is not allowed for embedded storage")
		}
	case ProfilePostgres:
		if options.PostgresDSN == "" {
			return OpenOptions{}, errors.New("PostgreSQL connection is required for postgres storage")
		}
		if options.SQLitePath != "" {
			return OpenOptions{}, errors.New("SQLite path is not allowed for postgres storage")
		}
	default:
		return OpenOptions{}, errors.New("unsupported storage profile")
	}
	return options, nil
}
