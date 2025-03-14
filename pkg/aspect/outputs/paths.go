/*
 * Copyright 2025 Aspect Build Systems, Inc.
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

package outputs

func unescape(path string) string {
	var result []rune
	runes := []rune(path)

	for i := 0; i < len(runes); i++ {
		if runes[i] == '\\' && i+1 < len(runes) {
			// Based on https://github.com/bazelbuild/bazel/blob/aa47fd1de7da398adb6e71b6122ced23c067a30b/src/main/tools/build-runfiles.cc#L112-L130
			// Introduced in bazel 7.4.0: https://github.com/bazelbuild/bazel/pull/23912
			switch runes[i+1] {
			case 's':
				result = append(result, ' ')
			case 'n':
				result = append(result, '\n')
			case 'b':
				result = append(result, '\\')
			case '\\':
				result = append(result, '\\')
			default:
				result = append(result, '\\', runes[i+1])
			}
			i++
		} else {
			result = append(result, runes[i])
		}
	}
	return string(result)
}
