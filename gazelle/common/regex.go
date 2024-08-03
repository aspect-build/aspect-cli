package gazelle

import (
	"regexp"
	"sync"
)

// A cache of parsed regex strings
var regexCache = make(map[string]*regexp.Regexp)
var regexMutex sync.Mutex

func ParseRegex(regexStr string) (*regexp.Regexp, error) {
	regexMutex.Lock()
	defer regexMutex.Unlock()

	if regexCache[regexStr] == nil {
		re, err := regexp.Compile(regexStr)
		if err != nil {
			return nil, err
		}

		regexCache[regexStr] = re
	}

	return regexCache[regexStr], nil
}
