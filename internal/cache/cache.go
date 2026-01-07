package cache

import (
	"time"

	gocache "github.com/patrickmn/go-cache"
)

type Cache struct {
	cache   *gocache.Cache
	enabled bool
}

func New(enabled bool) *Cache {
	return &Cache{
		cache:   gocache.New(5*time.Minute, 10*time.Minute),
		enabled: enabled,
	}
}

func (c *Cache) Get(key string) (interface{}, bool) {
	if !c.enabled {
		return nil, false
	}
	return c.cache.Get(key)
}

func (c *Cache) Set(key string, value interface{}, ttl time.Duration) {
	if !c.enabled {
		return
	}
	c.cache.Set(key, value, ttl)
}

func (c *Cache) SetDefault(key string, value interface{}) {
	if !c.enabled {
		return
	}
	c.cache.SetDefault(key, value)
}

func (c *Cache) Delete(key string) {
	if !c.enabled {
		return
	}
	c.cache.Delete(key)
}

func (c *Cache) Clear() {
	if !c.enabled {
		return
	}
	c.cache.Flush()
}
