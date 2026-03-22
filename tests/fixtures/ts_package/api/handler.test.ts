import { handleLogin, handleProfile, handleHealthCheck } from './handler';

describe('handler', () => {
    test('handleLogin returns token', async () => {
        const req = new Request('http://localhost/login', {
            method: 'POST',
            body: JSON.stringify({ username: 'admin', password: 'secret' }),
        });
        const res = await handleLogin(req);
        expect(res.status).toBe(200);
    });

    test('handleProfile returns user', async () => {
        const req = new Request('http://localhost/profile', {
            headers: { Authorization: 'valid-token' },
        });
        const res = await handleProfile(req);
        expect(res.status).toBe(200);
    });

    test('handleHealthCheck returns ok', () => {
        const res = handleHealthCheck();
        expect(res.status).toBe(200);
    });
});
