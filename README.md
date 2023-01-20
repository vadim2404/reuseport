# SO_REUSEPORT Example

This is an experiment of graceful shutdown.

Situation:

We have `N` (`N > 1`) servers that provide some API via TCP. And we want to release the new version of this API. On other hand, we don't want to disconnect current active sessions.

What's the idea?

1. Create 2+ listeners that shares the same port.
1. Listeners handles a SIGHUP
1. On receiving such signal, listener stops accepting new clients and waits until existing clients finish their work.


```bash
console-1 > cargo run
Process 62656 running
Process 62656 listening on: 127.0.0.1:8080

console-2 > cargo run
Process 66623 running
Process 66623 listening on: 127.0.0.1:8080

console-3 > telnet 127.0.0.1 8080
Trying 127.0.0.1...
Connected to localhost.
Escape character is '^]'
test
test
: from 66623 // <-------  Received it from console-2

console-4 > telnet 127.0.0.1 8080
Trying 127.0.0.1...
Connected to localhost.
Escape character is '^]'.
test
test
: from 66623 // <------- Again, it came from console-2

console-5 > kill -s HUP 66623 // <----- lets kill console-2

console-2 > 
Process 66623 accepted connection from: 127.0.0.1:58112
Process 66623 accepted connection from: 127.0.0.1:58115
Process 66623 received hangup signal
process 66623 closing tasks: 2

console-3 > fff
fff
: from 66623 // <--- it still works

console-5 > telnet 127.0.0.1 8080
Trying 127.0.0.1...
Connected to localhost.
Escape character is '^]'.
fff
fff
: from 62656 // <--- if trying to establish a new connection, it goes to console-1

console-3 > ^]
telnet> \q
Connection closed. // <-- close console-3 

console-4 > ^]
telnet> \q
Connection closed. // <-- and console-4

console-2 > ❯ kill -s HUP 66623
kill: kill 66623 failed: no such process // <-- The process terminates because all its clients have finished their job

console-5 > ^]
telnet> \q
Connection closed. // <-- Kill clients from console-1

console-5 > kill -s HUP 62656 // <-- and close the console-1

console-1 > Process 62656 received hangup signal
process 62656 closing tasks: 1 // <-- it is closed
``` 
