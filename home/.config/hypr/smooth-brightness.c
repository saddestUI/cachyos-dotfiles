#define _POSIX_C_SOURCE 200809L
#define _DEFAULT_SOURCE

#include <errno.h>
#include <math.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

static const char *BRIGHTNESS_PATH = "/sys/class/backlight/intel_backlight/brightness";
static const char *MAX_BRIGHTNESS_PATH = "/sys/class/backlight/intel_backlight/max_brightness";
static const char *DEVICE_NAME = "intel_backlight";

static int read_int_file(const char *path, long *out) {
    FILE *f = fopen(path, "r");
    if (!f) {
        return 0;
    }
    long v = 0;
    int ok = (fscanf(f, "%ld", &v) == 1);
    fclose(f);
    if (ok) {
        *out = v;
    }
    return ok;
}

static int write_int_file(const char *path, long val) {
    FILE *f = fopen(path, "w");
    if (!f) {
        return 0;
    }
    int ok = (fprintf(f, "%ld\n", val) > 0);
    fclose(f);
    return ok;
}

static void get_state_paths(char *pid_path, size_t pid_sz, char *target_path, size_t target_sz) {
    const char *runtime = getenv("XDG_RUNTIME_DIR");
    if (!runtime || !*runtime) {
        runtime = "/tmp";
    }
    snprintf(pid_path, pid_sz, "%s/smooth-brightness.pid", runtime);
    snprintf(target_path, target_sz, "%s/smooth-brightness.target", runtime);
}

static void set_raw_brightness(long raw) {
    char buf[32];
    snprintf(buf, sizeof(buf), "%ld", raw);
    pid_t pid = fork();
    if (pid == 0) {
        char *const argv[] = {
            "brightnessctl", "-d", (char *)DEVICE_NAME, "-q", "set", buf, NULL
        };
        execvp("brightnessctl", argv);
        _exit(127);
    } else if (pid > 0) {
        int status = 0;
        while (waitpid(pid, &status, 0) < 0 && errno == EINTR) {
        }
    }
}

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "Usage: %s <up [pct]|down [pct]|set <pct>>\n", argv[0]);
        return 1;
    }

    long current = 0, max_b = 0;
    if (!read_int_file(BRIGHTNESS_PATH, &current) || !read_int_file(MAX_BRIGHTNESS_PATH, &max_b) || max_b <= 0) {
        fprintf(stderr, "Failed to read backlight sysfs\n");
        return 1;
    }

    long min_b = (max_b * 2) / 100;
    if (min_b < 4) {
        min_b = 4;
    }
    if (min_b > max_b) {
        min_b = max_b;
    }

    char pid_path[512], target_path[512];
    get_state_paths(pid_path, sizeof(pid_path), target_path, sizeof(target_path));

    long base = current;
    long old_pid_val = 0;
    if (read_int_file(pid_path, &old_pid_val) && old_pid_val > 1) {
        pid_t old_pid = (pid_t)old_pid_val;
        if (old_pid != getpid() && kill(old_pid, 0) == 0) {
            long prev_target = 0;
            if (read_int_file(target_path, &prev_target) && prev_target >= min_b && prev_target <= max_b) {
                base = prev_target;
            }
            kill(old_pid, SIGTERM);
            usleep(2000);
        }
    }

    long target = current;
    if (strcmp(argv[1], "up") == 0) {
        double pct = (argc >= 3) ? atof(argv[2]) : 5.0;
        if (pct <= 0.0) {
            pct = 5.0;
        }
        long step = lround((pct / 100.0) * (double)max_b);
        if (step < 1) {
            step = 1;
        }
        target = base + step;
    } else if (strcmp(argv[1], "down") == 0) {
        double pct = (argc >= 3) ? atof(argv[2]) : 5.0;
        if (pct <= 0.0) {
            pct = 5.0;
        }
        long step = lround((pct / 100.0) * (double)max_b);
        if (step < 1) {
            step = 1;
        }
        target = base - step;
    } else if (strcmp(argv[1], "set") == 0) {
        if (argc < 3) {
            fprintf(stderr, "Usage: %s set <pct>\n", argv[0]);
            return 1;
        }
        double pct = atof(argv[2]);
        if (pct < 2.0) {
            pct = 2.0;
        }
        if (pct > 100.0) {
            pct = 100.0;
        }
        target = lround((pct / 100.0) * (double)max_b);
    } else {
        fprintf(stderr, "Unknown subcommand: %s\n", argv[1]);
        return 1;
    }

    if (target < min_b) {
        target = min_b;
    }
    if (target > max_b) {
        target = max_b;
    }

    pid_t self = getpid();
    write_int_file(target_path, target);
    write_int_file(pid_path, (long)self);

    long diff = labs(target - current);
    if (diff == 0) {
        set_raw_brightness(target);
    } else {
        int frames = (int)(diff < 10 ? 10 : (diff > 20 ? 20 : diff));
        double total_ms = 110.0 + (fmin((double)diff, (double)max_b) / (double)max_b) * 80.0;
        useconds_t frame_us = (useconds_t)((total_ms * 1000.0) / (double)frames);
        long last_written = current;

        for (int i = 1; i <= frames; i++) {
            double t = (double)i / (double)frames;
            double eased = 1.0 - pow(1.0 - t, 3.0);
            long val = current + lround((double)(target - current) * eased);
            if (val != last_written) {
                set_raw_brightness(val);
                last_written = val;
            }
            if (i < frames) {
                usleep(frame_us);
            }
        }
        if (last_written != target) {
            set_raw_brightness(target);
        }
    }

    long cur_pid = 0;
    if (read_int_file(pid_path, &cur_pid) && cur_pid == (long)self) {
        unlink(pid_path);
    }
    return 0;
}
