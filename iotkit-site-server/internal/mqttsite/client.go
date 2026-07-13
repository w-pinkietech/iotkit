package mqttsite

import (
	"context"
	"crypto/tls"
	"errors"
	"fmt"
	"log/slog"
	"net/url"
	"time"

	mqtt "github.com/eclipse/paho.mqtt.golang"
)

const recordsTopicFilter = "iotkit/v1/gateways/+/records"

type ClientConfig struct {
	BrokerURL     string
	ClientID      string
	Username      string
	Password      string
	TLSConfig     *tls.Config
	AllowInsecure bool
}

func Run(ctx context.Context, config ClientConfig, processor Processor, logger *slog.Logger) error {
	if err := config.validate(); err != nil {
		return err
	}
	if logger == nil {
		logger = slog.Default()
	}

	options := mqtt.NewClientOptions().
		AddBroker(config.BrokerURL).
		SetClientID(config.ClientID).
		SetUsername(config.Username).
		SetPassword(config.Password).
		SetCleanSession(true).
		SetAutoReconnect(true).
		SetConnectRetry(true).
		SetConnectRetryInterval(time.Second).
		SetOrderMatters(false)
	if config.TLSConfig != nil {
		options.SetTLSConfig(config.TLSConfig)
	}
	options.SetConnectionLostHandler(func(_ mqtt.Client, err error) {
		logger.Warn("MQTT connection lost", "error", err)
	})

	client := mqtt.NewClient(options)
	if token := client.Connect(); !token.WaitTimeout(15 * time.Second) {
		return errors.New("MQTT connect timed out")
	} else if err := token.Error(); err != nil {
		return fmt.Errorf("MQTT connect: %w", err)
	}
	defer client.Disconnect(250)

	handler := func(client mqtt.Client, message mqtt.Message) {
		err := processor.Process(context.Background(), message.Topic(), message.Payload(), func(topic string, payload []byte) error {
			token := client.Publish(topic, 1, false, payload)
			if !token.WaitTimeout(15 * time.Second) {
				return errors.New("accepted-through publish timed out")
			}
			return token.Error()
		})
		if err != nil {
			logger.Error("MQTT record batch not acknowledged", "topic", message.Topic(), "error", err)
		}
	}
	if token := client.Subscribe(recordsTopicFilter, 1, handler); !token.WaitTimeout(15 * time.Second) {
		return errors.New("MQTT subscribe timed out")
	} else if err := token.Error(); err != nil {
		return fmt.Errorf("MQTT subscribe: %w", err)
	}
	logger.Info("Site Server subscribed", "topic", recordsTopicFilter)

	<-ctx.Done()
	return nil
}

func (config ClientConfig) validate() error {
	if config.BrokerURL == "" || config.ClientID == "" {
		return errors.New("MQTT broker URL and client ID are required")
	}
	if config.Username == "" || config.Password == "" {
		return errors.New("MQTT username and password file content are required")
	}
	parsed, err := url.Parse(config.BrokerURL)
	if err != nil {
		return fmt.Errorf("parse MQTT broker URL: %w", err)
	}
	switch parsed.Scheme {
	case "ssl", "tls", "mqtts":
		if config.TLSConfig == nil {
			return errors.New("TLS broker requires TLS configuration")
		}
	case "tcp":
		if !config.AllowInsecure {
			return errors.New("plain MQTT requires explicit allow-insecure for local testing")
		}
	default:
		return fmt.Errorf("unsupported MQTT broker URL scheme %q", parsed.Scheme)
	}
	return nil
}
