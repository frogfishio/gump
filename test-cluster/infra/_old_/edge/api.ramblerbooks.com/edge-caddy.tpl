job "caddy-sites" {
  datacenters = ["*"]
  type        = "system"

  # OPTIONAL: constrain to the node that runs Caddy (replace values to match your host)
  # constraint {
  #   attribute = "${attr.unique.hostname}"
  #   value     = "my-caddy-hostname"
  # }

  group "writer" {
    task "templater" {
      driver = "raw_exec"     # lets the template write to /etc/caddy
      config {
        # keep the process alive so the template keeps watching SD changes
        command = "/bin/sh"
        args    = ["-lc", "while :; do sleep 3600; done"]
      }

      template {
        # write straight to the include directory Caddy imports
        destination = "/etc/caddy/sites/rambler-api.caddy"
        perms       = "0644"
        # If needed on your host, set uid/gid to the caddy user/group:
        # uid = 0
        # gid = 0

        # When upstreams change, reload Caddy gracefully
        change_mode = "script"
        change_script {
          command = "/bin/sh"
          args    = ["-lc", "systemctl reload caddy"]
          timeout = "10s"
          fail_on_error = true
        }

        data = <<-CADDY
          api.ramblerbooks.com {
            encode zstd gzip

            @hc path /healthz
            respond @hc 200

            # Dynamic upstreams from Nomad's native catalog:
            reverse_proxy {{- range nomadService "rambler-api" -}}{{ .Address }}:{{ .Port }} {{ end }} {
              health_uri /live
              health_interval 10s
              health_timeout 2s
              lb_policy random
              fail_duration 30s
            }

            log {
              output stdout
              format console
            }
          }
        CADDY
      }
    }
  }
}