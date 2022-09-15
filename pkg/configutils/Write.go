package configutils

import (
	"github.com/spf13/viper"
)

func Write(v *viper.Viper) error {
	// Workaround for issue with WriteConfig
	// https://github.com/spf13/viper/issues/433
	var err error
	if err = v.WriteConfig(); err != nil {
		if _, ok := err.(viper.ConfigFileNotFoundError); ok {
			err = v.SafeWriteConfig()
		}
	}
	return err
}
