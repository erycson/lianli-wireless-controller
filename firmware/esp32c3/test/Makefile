CC ?= gcc
CFLAGS ?= -std=c11 -O2 -Wall
INC = -I../main -I../components/esperanto/include

test: test_host
	./test_host

test_host: test_host.c ../main/lianli.c ../components/esperanto/helpers.c
	$(CC) $(CFLAGS) $(INC) -o $@ $^

clean:
	rm -f test_host

.PHONY: test clean
