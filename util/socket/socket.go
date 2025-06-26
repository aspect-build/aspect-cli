package socket

type Socket[S, R interface{}] interface {
	Send(cmd S) error
	Recv() (R, error)
	Close() error
}
