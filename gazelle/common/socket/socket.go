package socket

type Socket[S, R interface{}] interface {
	Send(cmd S) error
	Recv() (R, error)
	Close() error
}

type Server[S, R interface{}] interface {
	Socket[S, R]
	Serve(path string) error
	Accept() error
	HasConnection() bool
}
