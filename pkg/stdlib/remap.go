/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package stdlib

import (
	"io/fs"
	"net"
)

type FSFileInfo = fs.FileInfo

type NetAddr = net.Addr
type NetListener = net.Listener
