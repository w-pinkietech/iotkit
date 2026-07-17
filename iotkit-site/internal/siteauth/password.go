package siteauth

import (
	"crypto/rand"
	"crypto/subtle"
	"encoding/base64"
	"errors"
	"fmt"
	"strings"
	"unicode/utf8"

	"golang.org/x/crypto/argon2"
)

const (
	argonMemory      = 64 * 1024
	argonIterations  = 3
	argonParallelism = 1
	argonSaltBytes   = 16
	argonHashBytes   = 32
)

type argonParameters struct {
	memory      uint32
	iterations  uint32
	parallelism uint8
}

func HashPassword(password string) (string, error) {
	if err := ValidatePassword(password); err != nil {
		return "", err
	}
	salt := make([]byte, argonSaltBytes)
	if _, err := rand.Read(salt); err != nil {
		return "", errors.New("generate password salt")
	}
	hash := argon2.IDKey(
		[]byte(password),
		salt,
		argonIterations,
		argonMemory,
		argonParallelism,
		argonHashBytes,
	)
	return fmt.Sprintf(
		"$argon2id$v=%d$m=%d,t=%d,p=%d$%s$%s",
		argon2.Version,
		argonMemory,
		argonIterations,
		argonParallelism,
		base64.RawStdEncoding.EncodeToString(salt),
		base64.RawStdEncoding.EncodeToString(hash),
	), nil
}

func VerifyPassword(encoded, password string) (bool, bool, error) {
	parameters, salt, expected, err := parseArgon2ID(encoded)
	if err != nil {
		return false, false, err
	}
	actual := argon2.IDKey(
		[]byte(password),
		salt,
		parameters.iterations,
		parameters.memory,
		parameters.parallelism,
		uint32(len(expected)),
	)
	ok := subtle.ConstantTimeCompare(actual, expected) == 1
	needsRehash := ok && (parameters.memory != argonMemory ||
		parameters.iterations != argonIterations ||
		parameters.parallelism != argonParallelism ||
		len(salt) != argonSaltBytes ||
		len(expected) != argonHashBytes)
	return ok, needsRehash, nil
}

func ValidatePassword(password string) error {
	if !utf8.ValidString(password) {
		return errors.New("password must be valid UTF-8")
	}
	length := utf8.RuneCountInString(password)
	if length < 12 || length > 128 {
		return errors.New("password must be between 12 and 128 characters")
	}
	return nil
}

func NormalizeLoginID(loginID string) (string, error) {
	normalized := strings.ToLower(loginID)
	if len(normalized) < 3 || len(normalized) > 64 {
		return "", errors.New("login ID must be between 3 and 64 characters")
	}
	for _, character := range normalized {
		if character >= 'a' && character <= 'z' ||
			character >= '0' && character <= '9' ||
			character == '.' || character == '_' || character == '-' {
			continue
		}
		return "", errors.New("login ID contains an unsupported character")
	}
	return normalized, nil
}

func parseArgon2ID(encoded string) (argonParameters, []byte, []byte, error) {
	parts := strings.Split(encoded, "$")
	if len(parts) != 6 || parts[0] != "" || parts[1] != "argon2id" {
		return argonParameters{}, nil, nil, errors.New("invalid password hash encoding")
	}
	var version int
	if _, err := fmt.Sscanf(parts[2], "v=%d", &version); err != nil || version != argon2.Version {
		return argonParameters{}, nil, nil, errors.New("unsupported password hash version")
	}
	var parameters argonParameters
	if _, err := fmt.Sscanf(
		parts[3],
		"m=%d,t=%d,p=%d",
		&parameters.memory,
		&parameters.iterations,
		&parameters.parallelism,
	); err != nil {
		return argonParameters{}, nil, nil, errors.New("invalid password hash parameters")
	}
	if parameters.memory < 8*1024 || parameters.memory > 256*1024 ||
		parameters.iterations < 1 || parameters.iterations > 10 ||
		parameters.parallelism < 1 || parameters.parallelism > 8 {
		return argonParameters{}, nil, nil, errors.New("password hash parameters are outside supported bounds")
	}
	salt, err := base64.RawStdEncoding.Strict().DecodeString(parts[4])
	if err != nil || len(salt) < 16 || len(salt) > 64 {
		return argonParameters{}, nil, nil, errors.New("invalid password hash salt")
	}
	hash, err := base64.RawStdEncoding.Strict().DecodeString(parts[5])
	if err != nil || len(hash) < 16 || len(hash) > 64 {
		return argonParameters{}, nil, nil, errors.New("invalid password hash value")
	}
	return parameters, salt, hash, nil
}
