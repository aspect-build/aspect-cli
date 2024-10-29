package gazelle

import (
	"regexp"
	"sync"
)

// A cache of parsed regex strings
var regexCache = make(map[string]*regexp.Regexp)
var regexMutex sync.Mutex

func ParseRegex(regexStr string) *regexp.Regexp {
	regexMutex.Lock()
	defer regexMutex.Unlock()

	if regexCache[regexStr] == nil {
		regexCache[regexStr] = regexp.MustCompile(regexStr)
	}

	return regexCache[regexStr]
}
