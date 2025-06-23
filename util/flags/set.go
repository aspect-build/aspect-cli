package flags

import "strings"

/**
 * Parse a set of flag modifications and apply them to a base set of flag values.
 *
 * This should align with the bazel behaviour for arguments such as `--modify_execution_info`
 * where for a given set ("base") each argument can override, add or append to the set.
 */
func ParseSet(base []string, args []string) []string {
	// The flags to be returned, initialized with the base flags.
	resultSet := make(map[string]bool)
	for _, val := range base {
		resultSet[val] = true
	}

	for _, val := range args {
		parts := strings.Split(val, ",")
		for _, part := range parts {
			if part == "" { // Handle empty strings from "a,,b"
				continue
			}

			if strings.HasPrefix(part, "+") {
				resultSet[strings.TrimPrefix(part, "+")] = true
			} else if strings.HasPrefix(part, "-") {
				delete(resultSet, strings.TrimPrefix(part, "-"))
			} else {
				resultSet = make(map[string]bool) // Reset the set
				resultSet[part] = true
			}
		}
	}

	res := make([]string, 0, len(resultSet))

	// Maintain the original order of items from the base set
	for _, k := range base {
		if resultSet[k] {
			res = append(res, k)
			resultSet[k] = false // Mark as processed
		}
	}

	// Add any new items that were added
	for k, v := range resultSet {
		if v {
			res = append(res, k)
		}
	}

	return res
}
