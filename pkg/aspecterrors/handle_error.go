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

package aspecterrors

import (
	"errors"
	"fmt"
	"os"
)

// Output information about the provided error and terminate the process. This should only be used
// in an application's main function or equivalent.
func HandleError(err error) {
	var exitErr *ExitError
	if errors.As(err, &exitErr) {
		if exitErr.Err != nil {
			fmt.Fprintln(os.Stderr, "Error:", err)
		}
		os.Exit(exitErr.ExitCode)
	}

	fmt.Fprintln(os.Stderr, "Error:", err)
	os.Exit(1)
}
