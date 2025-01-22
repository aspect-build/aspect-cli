package runfiles

// Unescape the behavior of the C++ implementation
// https://github.com/bazelbuild/bazel/blob/release-8.0.1/src/main/tools/build-runfiles.cc#L107
func Unescape(path string) string {
	var result []rune
	runes := []rune(path)

	for i := 0; i < len(runes); i++ {
		if runes[i] == '\\' && i+1 < len(runes) {
			switch runes[i+1] {
			case 's':
				result = append(result, ' ')
			case 'n':
				result = append(result, '\n')
			case 'b':
				result = append(result, '\\')
			default:
				// For escaped backslash (\\), output single backslash
				if runes[i+1] == '\\' {
					result = append(result, '\\')
				} else {
					// For any other escaped character, preserve both the backslash and the character
					result = append(result, '\\', runes[i+1])
				}
			}
			i++
		} else {
			result = append(result, runes[i])
		}
	}
	return string(result)
}
