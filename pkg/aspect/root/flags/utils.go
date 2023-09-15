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

func AddFlagToCommand(command []string, flag string) []string {
	result := make([]string, 0, len(command)+1)
	for i, c := range command {
		if c == "--" {
			// inject the flag right before a double dash if it exists
			result = append(result, flag)
			result = append(result, command[i:len(command)]...)
			return result
		}
		result = append(result, c)
	}
	// if no double dash then add the flag at the end of the command
	result = append(result, flag)
	return result
}
