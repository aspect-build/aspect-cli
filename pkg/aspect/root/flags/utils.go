/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

package flags

import "strings"

// Separates flags from a list of arguments.
// Returns a list of flags and a list of remaining args
func SeparateFlagsFromArgs(args []string) ([]string, []string) {
	flags := make([]string, 0, 64)
	remainingArgs := make([]string, 0, 64)

	for _, s := range args {
		if strings.HasPrefix(s, "-") {
			flags = append(flags, s)
		} else {
			remainingArgs = append(remainingArgs, s)
		}
	}
	return flags, remainingArgs
}
