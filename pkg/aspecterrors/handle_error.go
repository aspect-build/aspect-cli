/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
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
