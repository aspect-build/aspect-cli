/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package stdlib

import (
	"io/fs"
	"net"
)

type FSFileInfo = fs.FileInfo

type NetAddr = net.Addr
type NetListener = net.Listener
