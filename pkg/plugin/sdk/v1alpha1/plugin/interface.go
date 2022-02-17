/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package plugin

import (
	buildeventstream "aspect.build/cli/bazel/buildeventstream/proto"
	"aspect.build/cli/pkg/ioutils"
)

// Plugin determines how an aspect Plugin should be implemented.
type Plugin interface {
	BEPEventCallback(event *buildeventstream.BuildEvent) error
	PostBuildHook(
		isInteractiveMode bool,
		promptRunner ioutils.PromptRunner,
	) error
}
