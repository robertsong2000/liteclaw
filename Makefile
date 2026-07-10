# Makefile for compiling helloworld.c

# The default target, which builds the executable
all: helloworld

# Rule to build the 'helloworld' executable
helloworld: helloworld.c
	gcc helloworld.c -o helloworld