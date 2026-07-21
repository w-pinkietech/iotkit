package store

import "strings"

func rebindPostgresPlaceholders(query string) string {
	var output strings.Builder
	output.Grow(len(query) + 8)
	placeholder := 1
	var quote byte
	for index := 0; index < len(query); index++ {
		character := query[index]
		if quote != 0 {
			output.WriteByte(character)
			if character == quote {
				if index+1 < len(query) && query[index+1] == quote {
					index++
					output.WriteByte(query[index])
					continue
				}
				quote = 0
			}
			continue
		}
		if character == '\'' || character == '"' {
			quote = character
			output.WriteByte(character)
			continue
		}
		if character == '?' {
			output.WriteByte('$')
			output.WriteString(intToDecimal(placeholder))
			placeholder++
			continue
		}
		output.WriteByte(character)
	}
	return output.String()
}

func intToDecimal(value int) string {
	if value == 0 {
		return "0"
	}
	var buffer [20]byte
	position := len(buffer)
	for value > 0 {
		position--
		buffer[position] = byte('0' + value%10)
		value /= 10
	}
	return string(buffer[position:])
}
