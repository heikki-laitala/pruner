package main

import (
	"fmt"
	"net/http"
)

func main() {
	handler := NewRouter()
	fmt.Println("Starting server on :8080")
	http.ListenAndServe(":8080", handler)
}
