job "docker-smoke" {
  datacenters = ["dc1"]
  type        = "service"

  group "smoke" {
    network {
      port "http" {
        to = 5678
      }
    }

    task "http-echo" {
      driver = "docker"

      config {
        image = "hashicorp/http-echo:1.0.0"
        args  = ["-listen", ":5678", "-text", "docker-ok"]
        ports = ["http"]
      }

      resources {
        cpu    = 100
        memory = 64
      }

      service {
        name = "docker-smoke"
        port = "http"

        check {
          type     = "http"
          path     = "/"
          interval = "10s"
          timeout  = "2s"
        }
      }
    }
  }
}