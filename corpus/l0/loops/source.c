int sum_to(int n) {
    int total = 0;
    int i;
    for (i = 0; i <= n; i++) {
        total += i;
    }
    return total;
}

int skip_three(int n) {
    int total = 0;
    for (int i = 0; i <= n; i++) {
        if (i == 3) {
            continue;
        }
        total += i;
    }
    return total;
}

int gcd(int a, int b) {
    while (b != 0) {
        int t = b;
        b = a % b;
        a = t;
    }
    return a;
}

int collatz_steps(int n) {
    int steps = 0;
    while (n != 1) {
        if (n % 2 == 0) {
            n = n / 2;
        } else {
            n = 3 * n + 1;
        }
        steps++;
    }
    return steps;
}

int factorial(int n) {
    int result = 1;
    for (int i = 2; i <= n; i++) {
        result *= i;
    }
    return result;
}

int nested_loops(int n) {
    int total = 0;
    for (int i = 0; i < n; i++) {
        for (int j = 0; j < n; j++) {
            total += i * j;
        }
    }
    return total;
}
