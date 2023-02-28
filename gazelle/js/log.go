package gazelle

import (
	"fmt"
	"log"
	"os"
	"path"

	"github.com/sirupsen/logrus"
)

var BazelLog = logrus.New()

func init() {
	BazelLog.Out = os.Stdout
	BazelLog.Level = logrus.WarnLevel

	// When running bazel tests output to a file in the test.outputs directory.
	// Do not write to stdout/err which gazelle tests depend on.
	if os.Getenv("BAZEL_TEST") == "1" {
		outputsDir := os.Getenv("TEST_UNDECLARED_OUTPUTS_DIR")
		testlogFile := path.Join(outputsDir, fmt.Sprintf("gazelle-%d.log", os.Getpid()))

		logfile, err := os.Create(testlogFile)
		if err != nil {
			log.Fatalln("failed to create test log file", err)
		}

		BazelLog.Out = logfile
		BazelLog.Level = logrus.DebugLevel
	}
}
