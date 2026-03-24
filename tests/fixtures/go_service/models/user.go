package models

import "errors"

type User struct {
	ID    int
	Email string
	Name  string
}

type UserStore struct {
	users []User
}

func NewUserStore() *UserStore {
	return &UserStore{
		users: []User{
			{ID: 1, Email: "admin@example.com", Name: "Admin"},
		},
	}
}

func (s *UserStore) FindByEmail(email string) (*User, error) {
	for _, u := range s.users {
		if u.Email == email {
			return &u, nil
		}
	}
	return nil, errors.New("user not found")
}

func (s *UserStore) FindByID(id int) (*User, error) {
	for _, u := range s.users {
		if u.ID == id {
			return &u, nil
		}
	}
	return nil, errors.New("user not found")
}
