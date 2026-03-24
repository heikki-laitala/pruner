package handlers_test

import (
	"net/http"
	"net/http/httptest"
	"testing"

	"example.com/service/handlers"
	"example.com/service/models"
)

func TestHandleLogin(t *testing.T) {
	store := models.NewUserStore()
	handler := handlers.NewAuthHandler(store)

	req := httptest.NewRequest(http.MethodPost, "/login", nil)
	rec := httptest.NewRecorder()

	handler.HandleLogin(rec, req)

	if rec.Code != http.StatusUnauthorized {
		t.Errorf("expected 401, got %d", rec.Code)
	}
}
