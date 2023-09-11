package log

import (
	"bufio"
	"fmt"
	"io"
	"log"
	"os"
	"path"
	"strings"
)

// A Level is a logging priority. Higher levels are more important.
type Level int8

const (
	TraceLevel Level = iota
	DebugLevel
	InfoLevel
	WarnLevel
	ErrorLevel
	FatalLevel
)

var level = WarnLevel

// Clone the default to align defaults.
var logger = log.New(log.Writer(), log.Prefix(), log.Flags())

func init() {
	// When running bazel tests output to a file in the test.outputs directory.
	if os.Getenv("BAZEL_TEST") == "1" {
		// A unique string per test target
		targetStr := os.Getenv("TEST_TARGET")
		targetStr = strings.ReplaceAll(targetStr, "/", "_")
		targetStr = strings.ReplaceAll(targetStr, ":", "__")
		targetStr = strings.Trim(targetStr, "_")

		// A unique log file per test target per process
		testlogFile := fmt.Sprintf("%d-%s.log", os.Getpid(), targetStr)

		// Output test logs to the bazel undeclared outputs
		outputsDir := os.Getenv("TEST_UNDECLARED_OUTPUTS_DIR")

		logfile, err := os.Create(path.Join(outputsDir, testlogFile))
		if err != nil {
			log.Fatalf("CLI failed to create test log file: %v\n", err)
		}

		logger.SetOutput(logfile)

		// Default to Debug for tests
		level = DebugLevel
	} else if os.Getenv("ASPECT_CLI_LOG_FILE") != "" {
		logfile, err := os.Create(os.Getenv("ASPECT_CLI_LOG_FILE"))
		if err != nil {
			log.Fatalf("CLI failed to create log file: %v\n", err)
		}

		log.Printf("CLI log file: %v\n", logfile.Name())

		logger.SetOutput(bufio.NewWriter(logfile))
	}

	// Override the default log level
	if os.Getenv("ASPECT_CLI_LOG_DEBUG") != "" {
		envLevel := strings.ToUpper(strings.TrimSpace(os.Getenv("ASPECT_CLI_LOG_DEBUG")))
		switch envLevel {
		case "TRACE":
			level = TraceLevel
		case "DEBUG":
			level = DebugLevel
		case "INFO":
			level = InfoLevel
		case "WARN":
			level = WarnLevel
		case "ERROR":
			level = ErrorLevel
		default:
			log.Fatalf("Invalid CLI log level: %s\n", envLevel)
		}

		log.Printf("CLI log level: %s\n", envLevel)
	}
}

func GetOutput() io.Writer {
	return logger.Writer()
}

func Tracef(format string, args ...interface{}) {
	if level > TraceLevel {
		return
	}
	logger.Printf(format, args...)
}

func Debugf(format string, args ...interface{}) {
	if level > DebugLevel {
		return
	}
	logger.Printf(format, args...)
}

func Infof(format string, args ...interface{}) {
	if level > InfoLevel {
		return
	}
	logger.Printf(format, args...)
}

func Warnf(format string, args ...interface{}) {
	if level > WarnLevel {
		return
	}
	logger.Printf(format, args...)
}

func Errorf(format string, args ...interface{}) {
	if level > ErrorLevel {
		return
	}
	logger.Printf(format, args...)
}

func Fatalf(format string, args ...interface{}) {
	logger.Fatalf(format+"\n", args...)
}

func IsLevelEnabled(l Level) bool {
	return level > l
}
