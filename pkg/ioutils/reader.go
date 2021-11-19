package ioutils

import (
	"bufio"
	"os"
	"strings"
)

var reader = bufio.NewReader(os.Stdin)

func ReadLine() string {
	path, err := reader.ReadString('\n')
	if err != nil {
		return ""
	}
	// convert CRLF to LF for Windows compatibility
	return strings.Replace(path, "\n", "", -1)
}
