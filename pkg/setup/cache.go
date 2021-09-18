package setup

import (
	"fmt"
	"os"
	"path"
	"runtime"
)

// LocateCacheFolder finds the canonical directory for writing temporary files based on the users operating system.
func LocateCacheFolder() (string, error) {
	// Possible return values are printed by `go tool dist list`
	switch runtime.GOOS {
	case "windows":
		return os.Getenv("LocalAppData"), nil
	case "darwin":
		if home := os.Getenv("HOME"); home != "" {
			return path.Join(home, "Library/Caches"), nil
		} else {
			return "", fmt.Errorf("$HOME must be set")
		}
	case "linux", "freebsd", "netbsd", "openbsd":
		if xdg := os.Getenv("XDG_CACHE_HOME"); xdg != "" {
			return xdg, nil
		} else if home := os.Getenv("HOME"); home != "" {
			return path.Join(home, ".cache"), nil
		} else {
			return "", fmt.Errorf("Either $HOME or $XDG_CACHE_HOME must be set")
		}
	}
	return "", fmt.Errorf("Operating system %s is not supported by aspect cli.", runtime.GOOS)
}
