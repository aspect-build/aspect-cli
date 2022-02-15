/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package client

import (
	goplugin "github.com/hashicorp/go-plugin"
)

// Factory hides the call to goplugin.NewClient.
type Factory interface {
	New(*goplugin.ClientConfig) Provider
}

func NewFactory() Factory {
	return &clientFactory{}
}

type clientFactory struct{}

// New calls the goplugin.NewClient with the given config.
func (*clientFactory) New(config *goplugin.ClientConfig) Provider {
	return goplugin.NewClient(config)
}

// Provider is an interface for goplugin.Client returned by
// goplugin.NewClient.
type Provider interface {
	Client() (goplugin.ClientProtocol, error)
	Kill()
}
