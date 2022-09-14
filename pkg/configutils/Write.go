package configutils

import (
	"log"

	"github.com/spf13/viper"
)

func Write(v *viper.Viper) error {
	// Workaround for issue with WriteConfig
	// https://github.com/spf13/viper/issues/433

	// DEBUG BEGIN
	log.Printf("*** CHUCK: START")
	// DEBUG END

	if err := v.WriteConfig(); err != nil {
		// DEBUG BEGIN
		log.Printf("*** CHUCK:  err: %+#v", err)
		// DEBUG END
		// var fileNotFoundError *viper.ConfigFileNotFoundError
		// if errors.As(err, &fileNotFoundError) {
		// if errors.Is(err, viper.ConfigFileNotFoundError) {
		// if _, ok := err.(*viper.ConfigFileNotFoundError); ok {
		if _, ok := err.(viper.ConfigFileNotFoundError); ok {
			// DEBUG BEGIN
			log.Printf("*** CHUCK: NotFoundErr")
			// DEBUG END
			if err = v.SafeWriteConfig(); err != nil {
				// DEBUG BEGIN
				log.Printf("*** CHUCK:  err: %v", err)
				// DEBUG END
				return err
			}
		}
	}
	return nil
}
