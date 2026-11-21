#define _POSIX_C_SOURCE 200809L

#include <ctype.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifndef DRUNCALC_TEST
#include <glib.h>
#include <rofi/helper.h>
#include <rofi/mode-private.h>
#include <rofi/mode.h>
#include <rofi/rofi-icon-fetcher.h>
#include <unistd.h>
#endif

typedef enum {
    CALC_EMPTY = 0,
    CALC_OK = 1,
    CALC_INVALID = 2
} CalcStatus;

typedef struct {
    double val;
    int is_pct;
} CalcVal;

typedef struct {
    const char *p;
    int err;
    int comma_is_arg;
} Parser;

static void skip_ws(Parser *ps) {
    while (*ps->p && isspace((unsigned char)*ps->p)) {
        ps->p++;
    }
}

static CalcVal parse_expr(Parser *ps);

static int is_ident_char(char c) {
    return isalnum((unsigned char)c) || c == '_';
}

static int peek_is_modulo(const char *after_pct) {
    const char *q = after_pct;
    while (*q && isspace((unsigned char)*q)) {
        q++;
    }
    if (!*q) {
        return 0;
    }
    if (isdigit((unsigned char)*q) || *q == '.' || *q == '(' || isalpha((unsigned char)*q) || *q == '_') {
        return 1;
    }
    return 0;
}

static CalcVal parse_number(Parser *ps) {
    CalcVal res = {0.0, 0};
    skip_ws(ps);
    const char *start = ps->p;
    char buf[128];
    size_t len = 0;
    int saw_dot = 0;
    int saw_digit = 0;

    while (*ps->p) {
        char c = *ps->p;
        if (isdigit((unsigned char)c)) {
            saw_digit = 1;
            if (len + 1 < sizeof(buf)) {
                buf[len++] = c;
            }
            ps->p++;
        } else if (c == '.' && !saw_dot) {
            saw_dot = 1;
            if (len + 1 < sizeof(buf)) {
                buf[len++] = '.';
            }
            ps->p++;
        } else if (c == ',' && !saw_dot && !ps->comma_is_arg && isdigit((unsigned char)ps->p[1])) {
            saw_dot = 1;
            if (len + 1 < sizeof(buf)) {
                buf[len++] = '.';
            }
            ps->p++;
        } else {
            break;
        }
    }

    if (saw_digit && (*ps->p == 'e' || *ps->p == 'E')) {
        const char *exp_p = ps->p + 1;
        if (*exp_p == '+' || *exp_p == '-') {
            exp_p++;
        }
        if (isdigit((unsigned char)*exp_p)) {
            if (len + 1 < sizeof(buf)) {
                buf[len++] = *ps->p++;
            }
            if (*ps->p == '+' || *ps->p == '-') {
                if (len + 1 < sizeof(buf)) {
                    buf[len++] = *ps->p++;
                }
            }
            while (isdigit((unsigned char)*ps->p)) {
                if (len + 1 < sizeof(buf)) {
                    buf[len++] = *ps->p++;
                } else {
                    ps->p++;
                }
            }
        }
    }

    if (!saw_digit || len == 0 || ps->p == start) {
        ps->err = 1;
        return res;
    }
    buf[len] = '\0';
    char *endptr = NULL;
    res.val = strtod(buf, &endptr);
    if (endptr == buf || !isfinite(res.val)) {
        ps->err = 1;
    }
    return res;
}

static int parse_func_args(Parser *ps, double *args, int min_args, int max_args, int *out_count) {
    skip_ws(ps);
    if (*ps->p != '(') {
        return 0;
    }
    ps->p++;
    int prev_comma_mode = ps->comma_is_arg;
    ps->comma_is_arg = (max_args > 1) ? 1 : 0;

    int count = 0;
    skip_ws(ps);
    if (*ps->p == ')') {
        ps->p++;
        ps->comma_is_arg = prev_comma_mode;
        *out_count = 0;
        return count >= min_args;
    }

    for (;;) {
        CalcVal v = parse_expr(ps);
        if (ps->err) {
            ps->comma_is_arg = prev_comma_mode;
            return 0;
        }
        if (count < max_args) {
            args[count++] = v.val;
        } else {
            ps->err = 1;
            ps->comma_is_arg = prev_comma_mode;
            return 0;
        }
        skip_ws(ps);
        if (*ps->p == ',') {
            ps->p++;
            continue;
        }
        if (*ps->p == ')') {
            ps->p++;
            break;
        }
        ps->err = 1;
        ps->comma_is_arg = prev_comma_mode;
        return 0;
    }

    ps->comma_is_arg = prev_comma_mode;
    *out_count = count;
    return count >= min_args && count <= max_args;
}

static CalcVal parse_primary(Parser *ps) {
    CalcVal res = {0.0, 0};
    skip_ws(ps);
    if (ps->err) {
        return res;
    }

    if (*ps->p == '(') {
        ps->p++;
        res = parse_expr(ps);
        skip_ws(ps);
        if (*ps->p != ')') {
            ps->err = 1;
            return res;
        }
        ps->p++;
        return res;
    }

    if (isdigit((unsigned char)*ps->p) || *ps->p == '.' || (*ps->p == ',' && !ps->comma_is_arg && isdigit((unsigned char)ps->p[1]))) {
        return parse_number(ps);
    }

    if (isalpha((unsigned char)*ps->p) || *ps->p == '_') {
        char name[64];
        size_t nlen = 0;
        while (is_ident_char(*ps->p)) {
            if (nlen + 1 < sizeof(name)) {
                name[nlen++] = (char)tolower((unsigned char)*ps->p);
            }
            ps->p++;
        }
        name[nlen] = '\0';

        skip_ws(ps);
        if (*ps->p == '(') {
            double a[8] = {0};
            int argc = 0;
            if (strcmp(name, "pow") == 0) {
                if (!parse_func_args(ps, a, 2, 2, &argc)) {
                    ps->err = 1;
                } else {
                    res.val = pow(a[0], a[1]);
                }
            } else if (strcmp(name, "min") == 0) {
                if (!parse_func_args(ps, a, 2, 8, &argc)) {
                    ps->err = 1;
                } else {
                    double m = a[0];
                    for (int i = 1; i < argc; i++) {
                        if (a[i] < m) m = a[i];
                    }
                    res.val = m;
                }
            } else if (strcmp(name, "max") == 0) {
                if (!parse_func_args(ps, a, 2, 8, &argc)) {
                    ps->err = 1;
                } else {
                    double m = a[0];
                    for (int i = 1; i < argc; i++) {
                        if (a[i] > m) m = a[i];
                    }
                    res.val = m;
                }
            } else {
                if (!parse_func_args(ps, a, 1, 1, &argc)) {
                    ps->err = 1;
                } else if (strcmp(name, "sqrt") == 0) {
                    if (a[0] < 0.0) ps->err = 1; else res.val = sqrt(a[0]);
                } else if (strcmp(name, "cbrt") == 0) {
                    res.val = cbrt(a[0]);
                } else if (strcmp(name, "abs") == 0) {
                    res.val = fabs(a[0]);
                } else if (strcmp(name, "sin") == 0) {
                    res.val = sin(a[0]);
                } else if (strcmp(name, "cos") == 0) {
                    res.val = cos(a[0]);
                } else if (strcmp(name, "tan") == 0) {
                    res.val = tan(a[0]);
                } else if (strcmp(name, "asin") == 0) {
                    if (a[0] < -1.0 || a[0] > 1.0) ps->err = 1; else res.val = asin(a[0]);
                } else if (strcmp(name, "acos") == 0) {
                    if (a[0] < -1.0 || a[0] > 1.0) ps->err = 1; else res.val = acos(a[0]);
                } else if (strcmp(name, "atan") == 0) {
                    res.val = atan(a[0]);
                } else if (strcmp(name, "ln") == 0) {
                    if (a[0] <= 0.0) ps->err = 1; else res.val = log(a[0]);
                } else if (strcmp(name, "log") == 0 || strcmp(name, "log10") == 0) {
                    if (a[0] <= 0.0) ps->err = 1; else res.val = log10(a[0]);
                } else if (strcmp(name, "log2") == 0) {
                    if (a[0] <= 0.0) ps->err = 1; else res.val = log2(a[0]);
                } else if (strcmp(name, "exp") == 0) {
                    res.val = exp(a[0]);
                } else if (strcmp(name, "floor") == 0) {
                    res.val = floor(a[0]);
                } else if (strcmp(name, "ceil") == 0) {
                    res.val = ceil(a[0]);
                } else if (strcmp(name, "round") == 0) {
                    res.val = round(a[0]);
                } else {
                    ps->err = 1;
                }
            }
            if (!isfinite(res.val)) {
                ps->err = 1;
            }
            return res;
        }

        if (strcmp(name, "pi") == 0) {
            res.val = 3.14159265358979323846;
            return res;
        }
        if (strcmp(name, "e") == 0) {
            res.val = 2.71828182845904523536;
            return res;
        }
        if (strcmp(name, "tau") == 0) {
            res.val = 6.28318530717958647692;
            return res;
        }

        ps->err = 1;
        return res;
    }

    ps->err = 1;
    return res;
}

static CalcVal parse_postfix(Parser *ps) {
    CalcVal res = parse_primary(ps);
    while (!ps->err) {
        skip_ws(ps);
        if (*ps->p == '!') {
            ps->p++;
            double n = round(res.val);
            if (res.val < 0.0 || fabs(res.val - n) > 1e-9 || n > 170.0) {
                ps->err = 1;
                return res;
            }
            double acc = 1.0;
            for (int i = 2; i <= (int)n; i++) {
                acc *= (double)i;
            }
            res.val = acc;
            res.is_pct = 0;
        } else if (*ps->p == '%' && !peek_is_modulo(ps->p + 1)) {
            ps->p++;
            res.val /= 100.0;
            res.is_pct = 1;
        } else {
            break;
        }
    }
    return res;
}

static CalcVal parse_unary(Parser *ps);

static CalcVal parse_power(Parser *ps) {
    CalcVal lhs = parse_postfix(ps);
    if (ps->err) {
        return lhs;
    }
    skip_ws(ps);
    if (*ps->p == '^' || (ps->p[0] == '*' && ps->p[1] == '*')) {
        ps->p += (*ps->p == '^') ? 1 : 2;
        CalcVal rhs = parse_unary(ps);
        if (ps->err) {
            return lhs;
        }
        lhs.val = pow(lhs.val, rhs.val);
        lhs.is_pct = 0;
        if (!isfinite(lhs.val)) {
            ps->err = 1;
        }
    }
    return lhs;
}

static CalcVal parse_unary(Parser *ps) {
    skip_ws(ps);
    if (*ps->p == '+') {
        ps->p++;
        return parse_unary(ps);
    }
    if (*ps->p == '-') {
        ps->p++;
        CalcVal v = parse_unary(ps);
        v.val = -v.val;
        return v;
    }
    return parse_power(ps);
}

static CalcVal parse_mul_div(Parser *ps) {
    CalcVal lhs = parse_unary(ps);
    while (!ps->err) {
        skip_ws(ps);
        if (ps->p[0] == '*' && ps->p[1] != '*') {
            ps->p++;
            CalcVal rhs = parse_unary(ps);
            lhs.val *= rhs.val;
            lhs.is_pct = 0;
        } else if (*ps->p == '/') {
            ps->p++;
            CalcVal rhs = parse_unary(ps);
            if (fabs(rhs.val) == 0.0) {
                ps->err = 1;
                return lhs;
            }
            lhs.val /= rhs.val;
            lhs.is_pct = 0;
        } else if (*ps->p == '%') {
            ps->p++;
            CalcVal rhs = parse_unary(ps);
            if (fabs(rhs.val) == 0.0) {
                ps->err = 1;
                return lhs;
            }
            lhs.val = fmod(lhs.val, rhs.val);
            lhs.is_pct = 0;
        } else {
            break;
        }
        if (!isfinite(lhs.val)) {
            ps->err = 1;
        }
    }
    return lhs;
}

static CalcVal parse_add_sub(Parser *ps) {
    CalcVal lhs = parse_mul_div(ps);
    while (!ps->err) {
        skip_ws(ps);
        if (*ps->p == '+') {
            ps->p++;
            CalcVal rhs = parse_mul_div(ps);
            if (rhs.is_pct) {
                lhs.val = lhs.val + lhs.val * rhs.val;
            } else {
                lhs.val += rhs.val;
            }
            lhs.is_pct = 0;
        } else if (*ps->p == '-') {
            ps->p++;
            CalcVal rhs = parse_mul_div(ps);
            if (rhs.is_pct) {
                lhs.val = lhs.val - lhs.val * rhs.val;
            } else {
                lhs.val -= rhs.val;
            }
            lhs.is_pct = 0;
        } else {
            break;
        }
        if (!isfinite(lhs.val)) {
            ps->err = 1;
        }
    }
    return lhs;
}

static CalcVal parse_expr(Parser *ps) {
    return parse_add_sub(ps);
}

static CalcStatus eval_calc_expr(const char *raw_expr, char *out_buf, size_t out_sz) {
    const char *p = raw_expr;
    while (*p && isspace((unsigned char)*p)) {
        p++;
    }
    if (!*p) {
        out_buf[0] = '\0';
        return CALC_EMPTY;
    }

    Parser ps = {p, 0, 0};
    CalcVal v = parse_expr(&ps);
    skip_ws(&ps);
    if (ps.err || *ps.p != '\0' || !isfinite(v.val)) {
        out_buf[0] = '\0';
        return CALC_INVALID;
    }

    double d = v.val;
    if (fabs(d) < 1e-14) {
        d = 0.0;
    }
    snprintf(out_buf, out_sz, "%.12g", d);
    if (strcmp(out_buf, "-0") == 0) {
        snprintf(out_buf, out_sz, "0");
    }
    return CALC_OK;
}

#ifndef DRUNCALC_TEST
extern Mode drun_mode;

typedef struct {
    int calc_active;
    CalcStatus status;
    char expr[256];
    char result[128];
    uint32_t calc_icon_uid;
} DrunCalcState;

static int druncalc_init(Mode *sw) {
    if (mode_get_private_data(sw) != NULL) {
        return TRUE;
    }
    if (!mode_init(&drun_mode)) {
        return FALSE;
    }
    DrunCalcState *st = g_malloc0(sizeof(DrunCalcState));
    mode_set_private_data(sw, st);
    return TRUE;
}

static void druncalc_destroy(Mode *sw) {
    DrunCalcState *st = (DrunCalcState *)mode_get_private_data(sw);
    if (st != NULL) {
        g_free(st);
        mode_set_private_data(sw, NULL);
        mode_destroy(&drun_mode);
    }
}

static unsigned int druncalc_get_num_entries(const Mode *sw) {
    (void)sw;
    unsigned int n = mode_get_num_entries(&drun_mode);
    return n > 0 ? n : 1u;
}

static char *druncalc_preprocess_input(Mode *sw, const char *input) {
    DrunCalcState *st = (DrunCalcState *)mode_get_private_data(sw);
    const char *p = input ? input : "";
    while (*p && isspace((unsigned char)*p)) {
        p++;
    }

    if (*p == '=') {
        st->calc_active = TRUE;
        const char *expr = p + 1;
        while (*expr && isspace((unsigned char)*expr)) {
            expr++;
        }
        snprintf(st->expr, sizeof(st->expr), "%s", expr);
        st->status = eval_calc_expr(expr, st->result, sizeof(st->result));
        return g_strdup("=");
    }

    st->calc_active = FALSE;
    st->status = CALC_EMPTY;
    st->expr[0] = '\0';
    st->result[0] = '\0';
    return mode_preprocess_input(&drun_mode, input);
}

static int druncalc_token_match(const Mode *sw, rofi_int_matcher **tokens, unsigned int index) {
    const DrunCalcState *st = (const DrunCalcState *)mode_get_private_data(sw);
    if (st && st->calc_active) {
        return index == 0 ? 1 : 0;
    }
    return mode_token_match(&drun_mode, tokens, index);
}

static char *druncalc_get_display_value(const Mode *sw, unsigned int selected_line, int *state,
                                        GList **attr_list, int get_entry) {
    const DrunCalcState *st = (const DrunCalcState *)mode_get_private_data(sw);
    if (st && st->calc_active && selected_line == 0) {
        *state = 0;
        if (!get_entry) {
            return NULL;
        }
        if (st->status == CALC_OK) {
            return g_strdup_printf("= %s   (Enter — скопировать)", st->result);
        }
        if (st->status == CALC_EMPTY) {
            return g_strdup("Введите выражение, например: = 2 + 2 * 5");
        }
        return g_strdup("Проверьте выражение (пример: = sqrt(144) + 15%)");
    }
    return mode_get_display_value(&drun_mode, selected_line, state, attr_list, get_entry);
}

static cairo_surface_t *druncalc_get_icon(const Mode *sw, unsigned int selected_line, int height) {
    DrunCalcState *st = (DrunCalcState *)mode_get_private_data(sw);
    if (st && st->calc_active && selected_line == 0) {
        if (st->calc_icon_uid == 0) {
            st->calc_icon_uid = rofi_icon_fetcher_query("accessories-calculator", height);
        }
        if (st->calc_icon_uid != 0) {
            cairo_surface_t *surf = rofi_icon_fetcher_get(st->calc_icon_uid);
            if (surf != NULL) {
                return cairo_surface_reference(surf);
            }
        }
        return NULL;
    }
    return mode_get_icon(&drun_mode, selected_line, (unsigned int)height);
}

static char *druncalc_get_completion(const Mode *sw, unsigned int selected_line) {
    const DrunCalcState *st = (const DrunCalcState *)mode_get_private_data(sw);
    if (st && st->calc_active && selected_line == 0) {
        if (st->status == CALC_OK) {
            return g_strdup(st->result);
        }
        return g_strdup("");
    }
    return mode_get_completion(&drun_mode, selected_line);
}

static char *druncalc_get_message(const Mode *sw) {
    const DrunCalcState *st = (const DrunCalcState *)mode_get_private_data(sw);
    if (st && st->calc_active) {
        return NULL;
    }
    return mode_get_message(&drun_mode);
}

static void copy_to_clipboard_and_notify(const char *expr, const char *result) {
    char *wl_argv[] = {"wl-copy", NULL};
    int stdin_fd = -1;
    if (g_spawn_async_with_pipes(NULL, wl_argv, NULL, G_SPAWN_SEARCH_PATH, NULL, NULL, NULL,
                                 &stdin_fd, NULL, NULL, NULL)) {
        if (stdin_fd >= 0) {
            size_t len = strlen(result);
            ssize_t off = 0;
            while (off < (ssize_t)len) {
                ssize_t w = write(stdin_fd, result + off, len - (size_t)off);
                if (w <= 0) {
                    break;
                }
                off += w;
            }
            close(stdin_fd);
        }
    }

    char *body = g_strdup_printf("%s = %s", expr, result);
    char *notify_argv[] = {"notify-send", "-a", "Калькулятор", "-t", "2500", "Скопировано в буфер", body, NULL};
    g_spawn_async(NULL, notify_argv, NULL, G_SPAWN_SEARCH_PATH, NULL, NULL, NULL, NULL);
    g_free(body);
}

static ModeMode druncalc_result(Mode *sw, int mretv, char **input, unsigned int selected_line) {
    DrunCalcState *st = (DrunCalcState *)mode_get_private_data(sw);
    if (mretv & MENU_CANCEL) {
        return MODE_EXIT;
    }
    if (mretv & MENU_NEXT) {
        return NEXT_DIALOG;
    }
    if (mretv & MENU_PREVIOUS) {
        return PREVIOUS_DIALOG;
    }

    if (st && st->calc_active) {
        if ((mretv & MENU_COMPLETE) && st->status == CALC_OK) {
            g_free(*input);
            *input = g_strdup_printf("= %s", st->result);
            return RELOAD_DIALOG;
        }
        if ((mretv & (MENU_OK | MENU_CUSTOM_INPUT | MENU_CUSTOM_ACTION)) && st->status == CALC_OK) {
            copy_to_clipboard_and_notify(st->expr, st->result);
            return MODE_EXIT;
        }
        return RELOAD_DIALOG;
    }

    return mode_result(&drun_mode, mretv, input, selected_line);
}

Mode mode = {
    .abi_version = ABI_VERSION,
    .name = "druncalc",
    .cfg_name_key = "display-druncalc",
    ._init = druncalc_init,
    ._get_num_entries = druncalc_get_num_entries,
    ._result = druncalc_result,
    ._destroy = druncalc_destroy,
    ._token_match = druncalc_token_match,
    ._get_display_value = druncalc_get_display_value,
    ._get_icon = (_mode_get_icon)druncalc_get_icon,
    ._get_completion = druncalc_get_completion,
    ._preprocess_input = druncalc_preprocess_input,
    ._get_message = druncalc_get_message,
    ._create = NULL,
    ._completer_result = NULL,
    .free = NULL,
    .type = MODE_TYPE_SWITCHER,
};
#else
int main(void) {
    struct {
        const char *expr;
        CalcStatus expected_status;
        const char *expected_out;
    } cases[] = {
        {"2+2", CALC_OK, "4"},
        {"3,5*2", CALC_OK, "7"},
        {"200+15%", CALC_OK, "230"},
        {"200-15%", CALC_OK, "170"},
        {"15%*200", CALC_OK, "30"},
        {"50%", CALC_OK, "0.5"},
        {"sqrt(144)+2^3", CALC_OK, "20"},
        {"pow(2,10)", CALC_OK, "1024"},
        {"min(3,7,1.5)", CALC_OK, "1.5"},
        {"max(3,7)", CALC_OK, "7"},
        {"sin(pi/2)", CALC_OK, "1"},
        {"0.1+0.2", CALC_OK, "0.3"},
        {"5!", CALC_OK, "120"},
        {"17%5", CALC_OK, "2"},
        {"2**3**2", CALC_OK, "512"},
        {"", CALC_EMPTY, ""},
        {"   ", CALC_EMPTY, ""},
        {"2 +", CALC_INVALID, ""},
        {"1/0", CALC_INVALID, ""},
    };
    int fails = 0;
    size_t n = sizeof(cases) / sizeof(cases[0]);
    for (size_t i = 0; i < n; i++) {
        char buf[128];
        CalcStatus st = eval_calc_expr(cases[i].expr, buf, sizeof(buf));
        if (st != cases[i].expected_status || (st == CALC_OK && strcmp(buf, cases[i].expected_out) != 0)) {
            fprintf(stderr, "FAIL [%s]: got status=%d out='%s', expected status=%d out='%s'\n",
                    cases[i].expr, st, buf, cases[i].expected_status, cases[i].expected_out);
            fails++;
        }
    }
    if (fails == 0) {
        printf("ALL_%zu_DRUNCALC_TESTS_PASSED\n", n);
    }
    return fails ? 1 : 0;
}
#endif
