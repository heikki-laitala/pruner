package handlers

import (
	"encoding/json"
	"net/http"

	"example.com/service/models"
)

type AuthHandler struct {
	userStore *models.UserStore
}

func NewAuthHandler(store *models.UserStore) *AuthHandler {
	return &AuthHandler{userStore: store}
}

func (h *AuthHandler) HandleLogin(w http.ResponseWriter, r *http.Request) {
	var req LoginRequest
	json.NewDecoder(r.Body).Decode(&req)

	user, err := h.userStore.FindByEmail(req.Email)
	if err != nil {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}

	token := generateToken(user)
	json.NewEncoder(w).Encode(map[string]string{"token": token})
}

func generateToken(user *models.User) string {
	return "token-" + user.Email
}

type LoginRequest struct {
	Email    string `json:"email"`
	Password string `json:"password"`
}
